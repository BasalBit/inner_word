//! Headless tests of the context menu: opening by right-click, dispatch to
//! the element that has the keyboard, keyboard operation, submenus, every
//! way of closing, the keyboard staying where it was, and placement.

use std::{cell::RefCell, rc::Rc, time::Duration};

use gpui::{
    Action, App, Bounds, Context, DismissEvent, Entity, FocusHandle, KeyBinding, Modifiers,
    MouseButton, MouseDownEvent, Pixels, Point, ScrollDelta, ScrollWheelEvent, TestAppContext,
    TouchPhase, VisualTestContext, Window, actions, div, point, prelude::*, px, size,
};
use writer_ui::{
    Button, CommandDescriptor, ContextMenu, ContextMenuItem, ContextMenuLayer, IconName, KeyHint,
    MenuAnchor, TextField, ToastLayer,
    context_menu::{OpenContextMenu, ROW_HEIGHT, SUBMENU_DELAY, WINDOW_MARGIN},
};

actions!(menu_test, [Rename, Duplicate, Delete, Disabled, Global]);

#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = menu_test, no_json)]
struct MoveTo {
    folder: usize,
}

type Log = Rc<RefCell<Vec<String>>>;

/// A window with two rows that open menus on right-click, a button, and a
/// focused element that handles the actions.
struct Host {
    focus: FocusHandle,
    log: Log,
    toasts: Option<Entity<ToastLayer>>,
    layers: usize,
}

fn row_items(log: &Log) -> Vec<ContextMenuItem> {
    let log = log.clone();
    vec![
        ContextMenuItem::action("Rename", Rename).icon(IconName::Document),
        ContextMenuItem::command(&CommandDescriptor::new("Duplicate", "Library", Duplicate)),
        ContextMenuItem::action("Disabled", Disabled).disabled(true),
        ContextMenuItem::submenu(
            "Move to",
            vec![
                ContextMenuItem::action("Drafts", MoveTo { folder: 1 }),
                ContextMenuItem::action("Archive", MoveTo { folder: 2 }).checked(true),
            ],
        ),
        ContextMenuItem::separator(),
        ContextMenuItem::new("Reveal", move |_, _| log.borrow_mut().push("reveal".into())),
        ContextMenuItem::action("Delete", Delete).danger(),
    ]
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let log = |entry: &'static str| {
            let log = self.log.clone();
            move || log.borrow_mut().push(entry.to_string())
        };
        let (rename, duplicate, delete, disabled) = (
            log("rename"),
            log("duplicate"),
            log("delete"),
            log("disabled"),
        );
        let move_log = self.log.clone();
        let button_log = self.log.clone();
        let items_log = self.log.clone();
        div()
            .size_full()
            .key_context("Host")
            .track_focus(&self.focus)
            .on_action(move |_: &Rename, _, _| rename())
            .on_action(move |_: &Duplicate, _, _| duplicate())
            .on_action(move |_: &Delete, _, _| delete())
            .on_action(move |_: &Disabled, _, _| disabled())
            .on_action(move |action: &MoveTo, _, _| {
                move_log
                    .borrow_mut()
                    .push(format!("move {}", action.folder))
            })
            .child(
                div()
                    .id("row")
                    .debug_selector(|| "row".into())
                    .w(px(200.))
                    .h(px(30.))
                    .on_mouse_down(
                        MouseButton::Right,
                        move |event: &MouseDownEvent, window, cx| {
                            ContextMenu::open(event.position, row_items(&items_log), window, cx);
                        },
                    ),
            )
            .child(
                div()
                    .id("row2")
                    .debug_selector(|| "row2".into())
                    .w(px(200.))
                    .h(px(30.))
                    .on_mouse_down(MouseButton::Right, |event: &MouseDownEvent, window, cx| {
                        ContextMenu::open(
                            event.position,
                            vec![ContextMenuItem::action("Only here", Rename)],
                            window,
                            cx,
                        );
                    }),
            )
            .child(
                div().debug_selector(|| "button".into()).child(
                    Button::new("button", "Button")
                        .on_click(move |_, _, _| button_log.borrow_mut().push("button".into())),
                ),
            )
            .children(self.toasts.clone())
            .children((0..self.layers).map(|_| ContextMenuLayer))
    }
}

fn host_window(
    cx: &mut TestAppContext,
    with_toasts: bool,
    layers: usize,
) -> (Entity<Host>, &mut VisualTestContext, Log) {
    super::init(cx);
    cx.update(|cx| {
        cx.bind_keys([
            KeyBinding::new("cmd-r", Rename, Some("Host")),
            KeyBinding::new("cmd-d", Duplicate, Some("Host")),
            KeyBinding::new("cmd-j", Global, None),
        ]);
    });
    let log = Log::default();
    let (host, cx) = cx.add_window_view({
        let log = log.clone();
        move |window, cx| {
            let focus = cx.focus_handle();
            window.focus(&focus);
            Host {
                focus,
                log,
                toasts: with_toasts.then(|| cx.new(|_| ToastLayer::new())),
                layers,
            }
        }
    });
    cx.simulate_resize(size(px(800.), px(600.)));
    cx.update(|window, cx| {
        window.activate_window();
        let log = log.clone();
        cx.on_action(move |_: &Global, _| log.borrow_mut().push("global".into()));
    });
    cx.run_until_parked();
    (host, cx, log)
}

fn menu(cx: &mut VisualTestContext) -> Option<Entity<ContextMenu>> {
    cx.update(|window, cx| ContextMenu::current(window, cx))
}

fn is_open(cx: &mut VisualTestContext) -> bool {
    menu(cx).is_some()
}

fn right_click(cx: &mut VisualTestContext, position: Point<Pixels>) {
    cx.simulate_mouse_down(position, MouseButton::Right, Modifiers::none());
    cx.simulate_mouse_up(position, MouseButton::Right, Modifiers::none());
}

fn item_centre(cx: &mut VisualTestContext, ix: usize) -> Point<Pixels> {
    let menu = menu(cx).expect("a menu is open");
    menu.read_with(cx, |menu, _| menu.item_bounds(ix))
        .expect("the menu was drawn")
        .center()
}

fn submenu_item_centre(cx: &mut VisualTestContext, ix: usize) -> Point<Pixels> {
    let menu = menu(cx).expect("a menu is open");
    menu.read_with(cx, |menu, _| menu.submenu_item_bounds(ix))
        .expect("the submenu was drawn")
        .center()
}

fn selected_title(cx: &mut VisualTestContext) -> Option<String> {
    let menu = menu(cx)?;
    menu.read_with(cx, |menu, _| {
        menu.selected_item().map(|item| item.title().to_string())
    })
}

fn focus_is_on_host(host: &Entity<Host>, cx: &mut VisualTestContext) -> bool {
    let focus = host.read_with(cx, |host, _| host.focus.clone());
    cx.update(|window, _| focus.is_focused(window))
}

const ROW: Point<Pixels> = point(px(40.), px(12.));
const ROW2: Point<Pixels> = point(px(40.), px(45.));
const FAR: Point<Pixels> = point(px(600.), px(500.));

#[gpui::test]
fn right_click_opens_and_a_click_reaches_the_focused_element(cx: &mut TestAppContext) {
    let (host, cx, log) = host_window(cx, false, 1);
    assert!(!is_open(cx));
    right_click(cx, ROW);
    let menu = menu(cx).expect("right-click opens the menu");
    let (labels, bounds, anchor) =
        menu.read_with(cx, |menu, _| (menu.labels(), menu.bounds(), menu.anchor()));
    assert_eq!(
        labels,
        [
            "Rename",
            "Duplicate",
            "Disabled",
            "Move to",
            "-",
            "Reveal",
            "Delete"
        ]
    );
    assert_eq!(anchor, MenuAnchor::Point(ROW));
    let bounds = bounds.expect("drawn by the layer");
    assert_eq!(bounds.origin, ROW, "the menu's corner is at the pointer");
    assert_eq!(cx.debug_bounds("context-menu"), Some(bounds));
    assert_eq!(
        cx.debug_bounds("context-menu-item-0").map(|b| b.origin.y),
        menu.read_with(cx, |menu, _| menu.item_bounds(0))
            .map(|b| b.origin.y),
        "item_bounds is where the row is drawn"
    );
    assert!(
        focus_is_on_host(&host, cx),
        "the menu does not take the keyboard"
    );
    // nothing is marked until the pointer or a key says so
    assert_eq!(selected_title(cx), None);

    let rename = item_centre(cx, 0);
    cx.simulate_mouse_move(rename, None, Modifiers::none());
    assert_eq!(selected_title(cx).as_deref(), Some("Rename"));
    cx.simulate_click(rename, Modifiers::none());
    assert!(!is_open(cx), "choosing closes the menu");
    assert_eq!(*log.borrow(), ["rename"]);
    assert!(focus_is_on_host(&host, cx));

    // a command descriptor's item runs the command's action
    right_click(cx, ROW);
    let duplicate = item_centre(cx, 1);
    cx.simulate_click(duplicate, Modifiers::none());
    assert_eq!(*log.borrow(), ["rename", "duplicate"]);

    // a closure item
    right_click(cx, ROW);
    let reveal = item_centre(cx, 5);
    cx.simulate_click(reveal, Modifiers::none());
    assert_eq!(*log.borrow(), ["rename", "duplicate", "reveal"]);
    assert!(!is_open(cx));

    // a disabled item cannot be chosen and keeps the menu open
    right_click(cx, ROW);
    let disabled = item_centre(cx, 2);
    cx.simulate_mouse_move(disabled, None, Modifiers::none());
    assert_eq!(selected_title(cx), None);
    cx.simulate_click(disabled, Modifiers::none());
    assert!(is_open(cx));
    assert_eq!(log.borrow().len(), 3);
}

#[gpui::test]
fn the_toast_layer_draws_the_menu(cx: &mut TestAppContext) {
    // what the app has: a toast layer and no menu layer of its own
    let (_host, cx, log) = host_window(cx, true, 0);
    right_click(cx, ROW);
    let rename = item_centre(cx, 0);
    cx.simulate_click(rename, Modifiers::none());
    assert_eq!(*log.borrow(), ["rename"]);
}

/// The layer's toast bounds are toasts only: the open context menu, which
/// the same layer draws, is never among them.
#[gpui::test]
fn toast_bounds_leave_the_open_menu_out(cx: &mut TestAppContext) {
    let (host, cx, _log) = host_window(cx, true, 0);
    let toasts = host.read_with(cx, |host, _| host.toasts.clone().unwrap());
    right_click(cx, ROW);
    let menu_bounds = menu(cx)
        .expect("a menu is open")
        .read_with(cx, |menu, _| menu.bounds())
        .expect("the menu was drawn");
    // a menu and no toast: nothing
    assert!(toasts.read_with(cx, |layer, _| layer.toast_bounds()).is_empty());

    cx.update(|_, cx| {
        toasts.update(cx, |layer, cx| {
            layer.push(writer_ui::Toast::new("First"), cx);
            layer.push(writer_ui::Toast::new("Second"), cx);
        })
    });
    cx.run_until_parked();
    assert!(is_open(cx), "the menu is still open");
    let bounds = toasts.read_with(cx, |layer, _| layer.toast_bounds());
    assert_eq!(
        bounds,
        vec![
            cx.debug_bounds("toast-0").expect("drawn"),
            cx.debug_bounds("toast-1").expect("drawn"),
        ]
    );
    assert!(!bounds.contains(&menu_bounds));
    assert!(bounds.iter().all(|toast| !toast.intersects(&menu_bounds)));
}

#[gpui::test]
fn two_layers_draw_one_menu(cx: &mut TestAppContext) {
    let (_host, cx, log) = host_window(cx, true, 2);
    for _ in 0..3 {
        right_click(cx, ROW);
        assert!(cx.debug_bounds("context-menu").is_some());
        // a second frame of the same menu is drawn as well
        let duplicate = item_centre(cx, 1);
        cx.simulate_mouse_move(duplicate, None, Modifiers::none());
        assert!(cx.debug_bounds("context-menu").is_some());
        let rename = item_centre(cx, 0);
        cx.simulate_click(rename, Modifiers::none());
    }
    assert_eq!(*log.borrow(), ["rename", "rename", "rename"]);
}

#[gpui::test]
fn without_a_layer_the_menu_still_works_from_the_keyboard(cx: &mut TestAppContext) {
    let (_host, cx, log) = host_window(cx, false, 0);
    right_click(cx, ROW);
    assert!(is_open(cx));
    assert!(cx.debug_bounds("context-menu").is_none());
    cx.simulate_keystrokes("down enter");
    assert_eq!(*log.borrow(), ["rename"]);
    right_click(cx, ROW);
    assert!(cx.update(|window, cx| ContextMenu::choose("Archive", window, cx)));
    assert_eq!(*log.borrow(), ["rename", "move 2"]);
    assert!(!cx.update(|window, cx| ContextMenu::choose("Rename", window, cx)));
}

#[gpui::test]
fn keyboard_navigation(cx: &mut TestAppContext) {
    let (host, cx, log) = host_window(cx, false, 1);
    // An interceptor like the editor's: acts on keys while "its" element
    // has the keyboard. It must not see the keys an open menu takes.
    let leaked = Log::default();
    cx.update(|_, cx| {
        let focus = host.read(cx).focus.clone();
        let leaked = leaked.clone();
        cx.intercept_keystrokes(move |event, window, _| {
            if focus.is_focused(window) {
                leaked.borrow_mut().push(event.keystroke.unparse());
            }
        })
        .detach();
    });

    right_click(cx, ROW);
    cx.simulate_keystrokes("down");
    assert_eq!(selected_title(cx).as_deref(), Some("Rename"));
    cx.simulate_keystrokes("down");
    assert_eq!(selected_title(cx).as_deref(), Some("Duplicate"));
    // the disabled item is skipped
    cx.simulate_keystrokes("down");
    assert_eq!(selected_title(cx).as_deref(), Some("Move to"));
    // so is the separator
    cx.simulate_keystrokes("down");
    assert_eq!(selected_title(cx).as_deref(), Some("Reveal"));
    cx.simulate_keystrokes("down down");
    assert_eq!(selected_title(cx).as_deref(), Some("Rename"), "wraps");
    cx.simulate_keystrokes("up");
    assert_eq!(selected_title(cx).as_deref(), Some("Delete"));
    cx.simulate_keystrokes("home");
    assert_eq!(selected_title(cx).as_deref(), Some("Rename"));
    cx.simulate_keystrokes("end");
    assert_eq!(selected_title(cx).as_deref(), Some("Delete"));
    cx.simulate_keystrokes("ctrl-p");
    assert_eq!(selected_title(cx).as_deref(), Some("Reveal"));
    cx.simulate_keystrokes("ctrl-n");
    assert_eq!(selected_title(cx).as_deref(), Some("Delete"));

    // type-ahead: one letter goes round the items that start with it
    cx.simulate_keystrokes("d");
    assert_eq!(selected_title(cx).as_deref(), Some("Duplicate"));
    cx.executor().advance_clock(Duration::from_secs(2));
    std::thread::sleep(Duration::from_millis(950));
    cx.simulate_keystrokes("d");
    assert_eq!(selected_title(cx).as_deref(), Some("Delete"));
    std::thread::sleep(Duration::from_millis(950));
    // several letters narrow down
    cx.simulate_keystrokes("r e v");
    assert_eq!(selected_title(cx).as_deref(), Some("Reveal"));
    std::thread::sleep(Duration::from_millis(950));
    // a key that is bound where the keyboard is does not run its action
    cx.simulate_keystrokes("m");
    assert_eq!(selected_title(cx).as_deref(), Some("Move to"));
    assert!(log.borrow().is_empty());
    assert!(is_open(cx));
    assert!(focus_is_on_host(&host, cx));

    cx.simulate_keystrokes("up up enter");
    assert_eq!(*log.borrow(), ["rename"]);
    assert!(!is_open(cx));
    assert!(focus_is_on_host(&host, cx));
    assert!(
        leaked.borrow().is_empty(),
        "keys taken by the menu reached another interceptor: {:?}",
        leaked.borrow()
    );

    // with the menu closed the keys are back where they were
    cx.simulate_keystrokes("cmd-d");
    assert_eq!(*log.borrow(), ["rename", "duplicate"]);
    assert_eq!(*leaked.borrow(), ["cmd-d"]);

    // space chooses too; up from nothing starts at the end
    right_click(cx, ROW);
    cx.simulate_keystrokes("up space");
    assert_eq!(*log.borrow(), ["rename", "duplicate", "delete"]);
}

#[gpui::test]
fn submenu_by_keyboard(cx: &mut TestAppContext) {
    let (host, cx, log) = host_window(cx, false, 1);
    right_click(cx, ROW);
    cx.simulate_keystrokes("m");
    assert_eq!(selected_title(cx).as_deref(), Some("Move to"));
    let menu = menu(cx).unwrap();
    assert_eq!(menu.read_with(cx, |menu, _| menu.open_submenu()), None);

    cx.simulate_keystrokes("right");
    assert_eq!(menu.read_with(cx, |menu, _| menu.open_submenu()), Some(3));
    assert_eq!(selected_title(cx).as_deref(), Some("Drafts"));
    let (root, sub) = menu.read_with(cx, |menu, _| (menu.bounds(), menu.submenu_bounds()));
    let (root, sub) = (root.unwrap(), sub.expect("the submenu is drawn"));
    assert!(sub.left() > root.right() - px(8.), "beside the menu");
    let parent_row = menu.read_with(cx, |menu, _| menu.item_bounds(3)).unwrap();
    assert_eq!(
        menu.read_with(cx, |menu, _| menu.submenu_item_bounds(0))
            .unwrap()
            .top(),
        parent_row.top(),
        "its first item is level with the row it belongs to"
    );

    cx.simulate_keystrokes("down");
    assert_eq!(selected_title(cx).as_deref(), Some("Archive"));
    cx.simulate_keystrokes("down");
    assert_eq!(selected_title(cx).as_deref(), Some("Drafts"));
    // left goes back to the menu, which stays open
    cx.simulate_keystrokes("left");
    assert_eq!(menu.read_with(cx, |menu, _| menu.open_submenu()), None);
    assert_eq!(selected_title(cx).as_deref(), Some("Move to"));
    // enter opens it too; escape leaves the submenu only
    cx.simulate_keystrokes("enter");
    assert_eq!(selected_title(cx).as_deref(), Some("Drafts"));
    cx.simulate_keystrokes("escape");
    assert!(is_open(cx));
    assert_eq!(menu.read_with(cx, |menu, _| menu.open_submenu()), None);

    cx.simulate_keystrokes("enter down enter");
    assert_eq!(*log.borrow(), ["move 2"]);
    assert!(!is_open(cx));
    assert!(focus_is_on_host(&host, cx));
}

#[gpui::test]
fn submenu_by_mouse(cx: &mut TestAppContext) {
    let (_host, cx, log) = host_window(cx, false, 1);
    right_click(cx, ROW);
    let menu = menu(cx).unwrap();
    let move_to = item_centre(cx, 3);

    // a rest on the row opens the submenu, not the first touch
    cx.simulate_mouse_move(move_to, None, Modifiers::none());
    assert_eq!(menu.read_with(cx, |menu, _| menu.open_submenu()), None);
    cx.executor()
        .advance_clock(SUBMENU_DELAY + Duration::from_millis(20));
    cx.run_until_parked();
    assert_eq!(menu.read_with(cx, |menu, _| menu.open_submenu()), Some(3));
    assert_eq!(
        menu.read_with(cx, |menu, _| menu.submenu_selected_index()),
        None
    );

    // resting on another row closes it again
    let rename = item_centre(cx, 0);
    cx.simulate_mouse_move(rename, None, Modifiers::none());
    assert_eq!(menu.read_with(cx, |menu, _| menu.open_submenu()), Some(3));
    cx.executor()
        .advance_clock(SUBMENU_DELAY + Duration::from_millis(20));
    cx.run_until_parked();
    assert_eq!(menu.read_with(cx, |menu, _| menu.open_submenu()), None);

    // a click opens it at once
    cx.simulate_click(move_to, Modifiers::none());
    assert_eq!(menu.read_with(cx, |menu, _| menu.open_submenu()), Some(3));
    assert!(is_open(cx));

    // moving into the submenu keeps it open, also past the delay
    let drafts = submenu_item_centre(cx, 0);
    cx.simulate_mouse_move(drafts, None, Modifiers::none());
    cx.executor().advance_clock(SUBMENU_DELAY * 3);
    cx.run_until_parked();
    assert_eq!(selected_title(cx).as_deref(), Some("Drafts"));
    assert_eq!(menu.read_with(cx, |menu, _| menu.selected_index()), Some(3));
    // a click inside the submenu is not a click outside the menu
    cx.simulate_click(drafts, Modifiers::none());
    assert_eq!(*log.borrow(), ["move 1"]);
    assert!(!is_open(cx));
}

#[gpui::test]
fn closing(cx: &mut TestAppContext) {
    let (host, cx, log) = host_window(cx, false, 1);
    let button = cx.debug_bounds("button").unwrap().center();
    let dismissed = Rc::new(RefCell::new(0));
    let open = |cx: &mut VisualTestContext| {
        right_click(cx, ROW);
        let menu = menu(cx).unwrap();
        let dismissed = dismissed.clone();
        cx.update(|_, cx| {
            cx.subscribe(&menu, move |_, _: &DismissEvent, _| {
                *dismissed.borrow_mut() += 1
            })
            .detach();
        });
    };

    // escape
    open(cx);
    cx.simulate_keystrokes("escape");
    assert!(!is_open(cx));
    assert_eq!(*dismissed.borrow(), 1);

    // a click outside closes the menu and does nothing else
    open(cx);
    cx.simulate_click(button, Modifiers::none());
    assert!(!is_open(cx));
    assert!(log.borrow().is_empty(), "the click went through: {log:?}");
    assert_eq!(*dismissed.borrow(), 2);
    // the next click is an ordinary one
    cx.simulate_click(button, Modifiers::none());
    assert_eq!(*log.borrow(), ["button"]);

    // a release that never comes (let go outside the window) is forgotten
    // at the next press: that click is an ordinary one too
    open(cx);
    cx.simulate_mouse_down(FAR, MouseButton::Left, Modifiers::none());
    assert!(!is_open(cx));
    cx.simulate_click(button, Modifiers::none());
    assert_eq!(*log.borrow(), ["button", "button"]);
    log.borrow_mut().pop();

    // a right-click outside closes it and opens the menu of what was clicked
    open(cx);
    right_click(cx, ROW2);
    assert_eq!(*dismissed.borrow(), 4);
    let labels = menu(cx).unwrap().read_with(cx, |menu, _| menu.labels());
    assert_eq!(labels, ["Only here"]);
    cx.simulate_keystrokes("escape");

    // scrolling what is underneath
    open(cx);
    let wheel = |position| ScrollWheelEvent {
        position,
        delta: ScrollDelta::Pixels(point(px(0.), px(-30.))),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    };
    // inside the menu: stays
    let inside = item_centre(cx, 1);
    cx.simulate_event(wheel(inside));
    assert!(is_open(cx));
    cx.simulate_event(wheel(FAR));
    assert!(!is_open(cx));
    assert_eq!(*dismissed.borrow(), 5);

    // a cmd chord closes the menu and does what it always does
    open(cx);
    cx.simulate_keystrokes("cmd-j");
    assert!(!is_open(cx));
    assert_eq!(*log.borrow(), ["button", "global"]);

    // ctrl-enter, the key that opens menus, closes an open one
    open(cx);
    cx.simulate_keystrokes("ctrl-enter");
    assert!(!is_open(cx));

    // opening another menu closes the open one (one at a time)
    open(cx);
    let before = *dismissed.borrow();
    cx.update(|window, cx| {
        ContextMenu::open(
            point(px(300.), px(300.)),
            vec![ContextMenuItem::action("Other", Rename)],
            window,
            cx,
        )
    });
    assert_eq!(*dismissed.borrow(), before + 1);
    assert_eq!(
        menu(cx).unwrap().read_with(cx, |menu, _| menu.labels()),
        ["Other"]
    );
    assert!(cx.update(ContextMenu::dismiss));
    assert!(!cx.update(ContextMenu::dismiss));

    // the window is no longer the active one
    open(cx);
    cx.deactivate_window();
    assert!(!is_open(cx));

    assert!(focus_is_on_host(&host, cx));
}

#[gpui::test]
fn stays_inside_the_window(cx: &mut TestAppContext) {
    let (_host, cx, _log) = host_window(cx, false, 1);
    let viewport = Bounds::new(point(px(0.), px(0.)), size(px(800.), px(600.)));
    let inside = |bounds: Bounds<Pixels>| {
        bounds.left() >= WINDOW_MARGIN
            && bounds.top() >= WINDOW_MARGIN
            && bounds.right() <= viewport.right() - WINDOW_MARGIN
            && bounds.bottom() <= viewport.bottom() - WINDOW_MARGIN
    };
    let items = || {
        vec![
            ContextMenuItem::action("Rename", Rename),
            ContextMenuItem::submenu(
                "Move to",
                vec![
                    ContextMenuItem::action("Drafts", MoveTo { folder: 1 }),
                    ContextMenuItem::action("Archive", MoveTo { folder: 2 }),
                ],
            ),
            ContextMenuItem::action("Delete", Delete),
        ]
    };
    let open_at = |cx: &mut VisualTestContext, anchor: MenuAnchor| {
        let menu = cx.update(|window, cx| ContextMenu::open(anchor, items(), window, cx));
        cx.run_until_parked();
        let bounds = menu.read_with(cx, |menu, _| menu.bounds()).unwrap();
        (menu, bounds)
    };

    // bottom right corner: the menu ends at the pointer instead of starting there
    let corner = point(px(795.), px(595.));
    let (menu, bounds) = open_at(cx, MenuAnchor::Point(corner));
    assert!(inside(bounds), "{bounds:?}");
    assert!(bounds.right() <= corner.x && bounds.bottom() <= corner.y);
    // its submenu has no room on the right and opens on the left
    cx.simulate_keystrokes("down down right");
    let sub = menu.read_with(cx, |menu, _| menu.submenu_bounds()).unwrap();
    assert!(inside(sub), "{sub:?}");
    assert!(sub.right() <= bounds.left() + px(8.), "{sub:?} {bounds:?}");
    // and it can be clicked there
    let archive = submenu_item_centre(cx, 1);
    cx.simulate_click(archive, Modifiers::none());
    assert!(!is_open(cx));

    // top left, outside the window
    let (_, bounds) = open_at(cx, MenuAnchor::Point(point(px(-20.), px(-20.))));
    assert_eq!(bounds.origin, point(WINDOW_MARGIN, WINDOW_MARGIN));

    // below a button, left edges aligned
    let button = Bounds::new(point(px(100.), px(50.)), size(px(60.), px(28.)));
    let (_, bounds) = open_at(cx, button.into());
    assert_eq!(bounds.left(), button.left());
    assert!(bounds.top() >= button.bottom());
    // no room below: above it
    let low = Bounds::new(point(px(100.), px(560.)), size(px(60.), px(28.)));
    let (_, bounds) = open_at(cx, low.into());
    assert!(inside(bounds));
    assert!(bounds.bottom() <= low.top());

    // a menu taller than the window is as tall as the window allows and scrolls
    let many: Vec<_> = (0..60)
        .map(|ix| ContextMenuItem::action(format!("Item {ix}"), MoveTo { folder: ix }))
        .collect();
    let menu =
        cx.update(|window, cx| ContextMenu::open(point(px(100.), px(300.)), many, window, cx));
    cx.run_until_parked();
    let bounds = menu.read_with(cx, |menu, _| menu.bounds()).unwrap();
    assert!(inside(bounds), "{bounds:?}");
    assert!(bounds.size.height > ROW_HEIGHT * 20.);
    cx.simulate_keystrokes("end");
    cx.run_until_parked();
    let last = menu.read_with(cx, |menu, _| menu.item_bounds(59)).unwrap();
    assert!(
        last.bottom() <= bounds.bottom() && last.top() >= bounds.top(),
        "the marked item is scrolled into view: {last:?} in {bounds:?}"
    );
}

#[gpui::test]
fn key_hints_come_from_the_keymap(cx: &mut TestAppContext) {
    let (_host, cx, _log) = host_window(cx, false, 1);
    cx.update(|_, cx| {
        let rename = ContextMenuItem::action("Rename", Rename);
        assert_eq!(rename.action_name(), Some("menu_test::Rename"));
        assert_eq!(
            KeyHint::for_action(rename.action_ref().unwrap(), cx).map(|hint| hint.text()),
            Some("⌘R".to_string())
        );
        let explicit =
            ContextMenuItem::new("Reveal", |_, _| {}).key_hint(KeyHint::parse("cmd-alt-r"));
        assert_eq!(explicit.action_name(), None);
        assert!(format!("{explicit:?}").contains("closure"));
        let from: ContextMenuItem =
            (&CommandDescriptor::new("Duplicate", "Library", Duplicate)).into();
        assert_eq!(from.title().as_ref(), "Duplicate");
        assert_eq!(from.action_name(), Some("menu_test::Duplicate"));
    });
    // an empty list still opens something that says so
    let menu = cx.update(|window, cx| {
        ContextMenu::open(
            point(px(10.), px(10.)),
            vec![ContextMenuItem::separator()],
            window,
            cx,
        )
    });
    assert_eq!(
        menu.read_with(cx, |menu, _| menu.labels()),
        ["No actions here"]
    );
    cx.simulate_keystrokes("down enter");
    assert!(is_open(cx));
    cx.simulate_keystrokes("escape");
    assert!(!is_open(cx));
}

#[gpui::test]
fn an_explicit_target_gets_the_actions(cx: &mut TestAppContext) {
    // the keyboard is in a text field; the menu belongs to the host
    let (host, cx, log) = host_window(cx, false, 1);
    let field = cx.update(|window, cx| cx.new(|cx| TextField::new(window, cx)));
    let _ = field;
    let target = host.read_with(cx, |host, _| host.focus.clone());
    cx.update(|window, cx| {
        window.blur();
        ContextMenu::open_for(
            &target,
            point(px(300.), px(300.)),
            vec![ContextMenuItem::action("Rename", Rename)],
            window,
            cx,
        )
        .update(cx, |menu, cx| menu.select_first(cx));
    });
    assert_eq!(selected_title(cx).as_deref(), Some("Rename"));
    cx.simulate_keystrokes("enter");
    assert_eq!(*log.borrow(), ["rename"]);
    let _ = OpenContextMenu;
    let _: fn(&mut App) = writer_ui::context_menu::init;
}

#[gpui::test]
fn a_menu_of_choices_opens_on_the_current_one(cx: &mut TestAppContext) {
    let (_host, cx, _log) = host_window(cx, false, 1);
    let choices = |checked: Option<usize>| {
        ["Small", "Medium", "Large"]
            .into_iter()
            .enumerate()
            .map(|(ix, title)| {
                ContextMenuItem::new(title, |_, _| {})
                    .checked(checked == Some(ix))
                    .disabled(ix == 0)
            })
            .collect::<Vec<_>>()
    };
    cx.update(|window, cx| {
        ContextMenu::open(point(px(300.), px(300.)), choices(Some(2)), window, cx)
            .update(cx, |menu, cx| menu.select_checked(cx));
    });
    assert_eq!(selected_title(cx).as_deref(), Some("Large"));
    cx.simulate_keystrokes("up");
    assert_eq!(selected_title(cx).as_deref(), Some("Medium"));
    cx.simulate_keystrokes("escape");

    // Nothing checked, or the checked item cannot be chosen: the first
    // item that can.
    for checked in [None, Some(0)] {
        cx.update(|window, cx| {
            ContextMenu::open(point(px(300.), px(300.)), choices(checked), window, cx)
                .update(cx, |menu, cx| menu.select_checked(cx));
        });
        assert_eq!(selected_title(cx).as_deref(), Some("Medium"));
        cx.simulate_keystrokes("escape");
    }
}
