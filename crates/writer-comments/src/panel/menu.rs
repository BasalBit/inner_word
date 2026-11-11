//! The context menus of the panel: what a right-click, the "more" button
//! of a card or `ctrl-enter` in the thread list offers.
//!
//! | On | Items |
//! |---|---|
//! | a thread (also a detached one) | Reply, Edit, Resolve or Reopen, Copy Text, the app's items, Delete |
//! | a reply | Edit Reply, Copy Text, the app's items, Delete Reply |
//! | the panel itself (header, footer, the room under the cards) | Add Comment on Selection, Show or Hide Resolved, Hide Comments, the app's items |
//! | the composer | Cut, Copy, Paste, Select All |
//!
//! The thread or reply a menu is opened on becomes the active thread and
//! the selected reply first, and the thread list takes the keyboard: the
//! menu is about what is marked, as a right-click on a row of a list works
//! everywhere. Every item of the panel's own is one of the panel's
//! actions. Choosing it dispatches that action at the thread list, so it
//! does exactly what its key does, and the menu shows that key, looked up
//! in the keymap by the menu widget. An item that cannot apply (anything
//! that changes a read-only document, Show Resolved without a resolved
//! thread) is shown disabled. The menu itself never takes the keyboard
//! (see `writer_ui::context_menu`).

use std::rc::Rc;

use gpui::{
    Action, App, Bounds, Context, DismissEvent, Focusable, Pixels, Subscription, Window, point, px,
};
use writer_core::{CommentId, ReplyId};
use writer_editor::ContextMenuRequest;
use writer_ui::{ContextMenu, ContextMenuItem, MenuAnchor};

use super::{CommentsPanel, Row};
use crate::actions::{
    AddComment, CopyComment, DeleteComment, EditComment, ReplyToThread, ToggleComments,
    ToggleResolved, ToggleShowResolved,
};

/// What a context menu of the panel was opened on. The app receives it in
/// its [menu extension](CommentsPanel::set_menu_extension) and decides what
/// to add.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommentMenuTarget {
    /// A thread card: the opening comment of the thread.
    Thread(CommentId),
    /// One reply of a thread.
    Reply(CommentId, ReplyId),
    /// The panel itself: its header, its footer, the room under the cards.
    Panel,
}

impl CommentMenuTarget {
    /// The thread the target is, or belongs to.
    pub fn thread(&self) -> Option<CommentId> {
        match *self {
            CommentMenuTarget::Thread(thread) | CommentMenuTarget::Reply(thread, _) => Some(thread),
            CommentMenuTarget::Panel => None,
        }
    }
}

pub(crate) type MenuExtension =
    Rc<dyn Fn(CommentMenuTarget, &mut Window, &mut App) -> Vec<ContextMenuItem>>;

/// Where the menus that are not opened at the pointer drop down from, as
/// the last frame drew it.
#[derive(Default)]
pub(crate) struct MenuAnchors {
    /// The "more" buttons that were drawn.
    pub(crate) more: Vec<(CommentMenuTarget, Bounds<Pixels>)>,
    /// The header of the panel.
    pub(crate) header: Option<Bounds<Pixels>>,
}

impl CommentsPanel {
    /// Lets the app add its own items to the menus of the panel.
    ///
    /// `extension` is called each time a menu is about to open, with what
    /// the menu is on, and returns the items to add; an empty list adds
    /// nothing. They are placed after the panel's own items and before the
    /// destructive one ("Delete"), between separators. The thread or reply
    /// is already the active thread and the selected reply when the
    /// extension runs, and it still is when an item runs. The panel is not
    /// being updated while the extension runs, so it may be read there.
    ///
    /// ```ignore
    /// panel.update(cx, |panel, _| {
    ///     panel.set_menu_extension(move |target, _window, _cx| match target {
    ///         CommentMenuTarget::Thread(thread) => vec![ContextMenuItem::new(
    ///             "Copy Link to Comment",
    ///             move |_window, cx| { /* something with `thread` */ },
    ///         )],
    ///         _ => Vec::new(),
    ///     });
    /// });
    /// ```
    ///
    /// An action item is dispatched at the thread list of the panel, so
    /// it travels from there up to the window root like one of the
    /// panel's own keys.
    pub fn set_menu_extension(
        &mut self,
        extension: impl Fn(CommentMenuTarget, &mut Window, &mut App) -> Vec<ContextMenuItem> + 'static,
    ) {
        self.menu_extension = Some(Rc::new(extension));
    }

    /// What the open menu of the panel is on, while one is open.
    pub fn menu_target(&self) -> Option<CommentMenuTarget> {
        self.menu_target
    }

    /// The panel's own items for `target`: those before the app's items,
    /// and those after them. Each is an action of the panel, dispatched at
    /// the thread list when it is chosen.
    fn own_menu_items(
        &self,
        target: CommentMenuTarget,
        cx: &Context<Self>,
    ) -> (Vec<ContextMenuItem>, Vec<ContextMenuItem>) {
        let writable = !self.read_only(cx);
        let item = |title: &'static str, action: Box<dyn Action>, enabled: bool| {
            ContextMenuItem::action_boxed(title, action).disabled(!enabled)
        };
        match target {
            CommentMenuTarget::Thread(thread) => {
                let resolved = self.thread(thread).is_some_and(|entry| entry.is_resolved());
                (
                    vec![
                        item("Reply", Box::new(ReplyToThread), writable),
                        item("Edit", Box::new(EditComment), writable),
                        item(
                            if resolved { "Reopen" } else { "Resolve" },
                            Box::new(ToggleResolved),
                            writable,
                        ),
                        ContextMenuItem::separator(),
                        item("Copy Text", Box::new(CopyComment), true),
                    ],
                    vec![item("Delete", Box::new(DeleteComment), writable).danger()],
                )
            }
            CommentMenuTarget::Reply(..) => (
                vec![
                    item("Edit Reply", Box::new(EditComment), writable),
                    item("Copy Text", Box::new(CopyComment), true),
                ],
                vec![item("Delete Reply", Box::new(DeleteComment), writable).danger()],
            ),
            CommentMenuTarget::Panel => (
                vec![
                    item(
                        "Add Comment on Selection",
                        Box::new(AddComment),
                        self.can_add_comment(cx),
                    ),
                    ContextMenuItem::separator(),
                    item(
                        if self.show_resolved {
                            "Hide Resolved"
                        } else {
                            "Show Resolved"
                        },
                        Box::new(ToggleShowResolved),
                        self.show_resolved || self.resolved_count() > 0,
                    ),
                    item("Hide Comments", Box::new(ToggleComments), true),
                ],
                Vec::new(),
            ),
        }
    }

    /// Opens the menu for `target` at `anchor`: a window position (the
    /// pointer of a right-click), or the bounds it drops down from. A
    /// thread or a reply becomes the active thread and the selected reply
    /// first, and the thread list takes the keyboard. With `from_keyboard`
    /// the first item is marked, so `enter` chooses it.
    ///
    /// The menu is opened right after the current update (the app's
    /// [extension](CommentsPanel::set_menu_extension) is asked then);
    /// [`CommentsPanel::menu_target`] says at once what it will be on.
    /// Nothing opens for a thread the panel does not list.
    pub fn open_menu(
        &mut self,
        target: CommentMenuTarget,
        anchor: impl Into<MenuAnchor>,
        from_keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let anchor = anchor.into();
        match target {
            CommentMenuTarget::Thread(thread) => {
                if !self.aim(thread, None, cx) {
                    return;
                }
                window.focus(&self.focus_handle);
            }
            CommentMenuTarget::Reply(thread, reply) => {
                if !self.aim(thread, Some(reply), cx) {
                    return;
                }
                window.focus(&self.focus_handle);
            }
            CommentMenuTarget::Panel => {}
        }
        let (mut items, after) = self.own_menu_items(target, cx);
        let extension = self.menu_extension.clone();
        let panel = cx.weak_entity();
        let list = self.focus_handle.clone();
        // The menu this one replaces reports its end after this update;
        // that must not take the new target away.
        self.menu_generation += 1;
        let generation = self.menu_generation;
        self.menu_target = Some(target);
        cx.notify();
        window.defer(cx, move |window, cx| {
            let extra = extension
                .map(|extension| extension(target, window, cx))
                .unwrap_or_default();
            // The widget drops separators that separate nothing.
            items.push(ContextMenuItem::separator());
            items.extend(extra);
            items.push(ContextMenuItem::separator());
            items.extend(after);
            // Dispatched at the thread list, whatever has the keyboard
            // when the item is chosen.
            let menu = ContextMenu::open_for(&list, anchor, items, window, cx);
            if from_keyboard {
                menu.update(cx, |menu, cx| menu.select_first(cx));
            }
            let subscription = cx.subscribe(&menu, {
                let panel = panel.clone();
                move |_, _: &DismissEvent, cx| {
                    panel
                        .update(cx, |panel, cx| {
                            if panel.menu_generation == generation {
                                panel.menu_target = None;
                                cx.notify();
                            }
                        })
                        .ok();
                }
            });
            panel
                .update(cx, |panel, _| panel.menu_subscription = Some(subscription))
                .ok();
        });
    }

    /// `ctrl-enter` in the thread list: the menu of the selected reply, or
    /// of the active thread, dropping down from its "more" button; without
    /// an active thread, the menu of the panel under its header.
    pub fn open_context_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = match (self.active, self.selected_reply) {
            (Some(thread), Some(reply)) => CommentMenuTarget::Reply(thread, reply),
            (Some(thread), None) => CommentMenuTarget::Thread(thread),
            (None, _) => CommentMenuTarget::Panel,
        };
        let anchor = self.menu_anchor(target);
        self.open_menu(target, anchor, true, window, cx);
    }

    /// Where the menu of `target` drops down from when it is not opened at
    /// the pointer: its "more" button if that is drawn, else the top of
    /// its card, else the header of the panel.
    pub(crate) fn menu_anchor(&self, target: CommentMenuTarget) -> MenuAnchor {
        let anchors = self.menu_anchors.borrow();
        if let Some((_, bounds)) = anchors.more.iter().find(|(drawn, _)| *drawn == target) {
            return MenuAnchor::Below(*bounds);
        }
        let card = target
            .thread()
            .and_then(|thread| self.row_index(Row::Thread(thread)))
            .and_then(|ix| self.list.bounds_for_item(ix));
        if let Some(card) = card {
            return MenuAnchor::Point(point(card.left() + px(24.), card.top() + px(24.)));
        }
        match anchors.header {
            Some(header) => MenuAnchor::Below(Bounds::new(
                point(header.left() + px(12.), header.top()),
                header.size,
            )),
            None => MenuAnchor::Point(point(px(0.), px(0.))),
        }
    }

    /// A right-click (or `ctrl-enter`) in the composer: the menu of a text
    /// field, with the composer's own cut, copy, paste and select all.
    pub(crate) fn open_composer_menu(
        &mut self,
        request: &ContextMenuRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use writer_editor::actions::{Copy, Cut, Paste, SelectAll};
        let composer = self.composer.read(cx);
        let target = composer.focus_handle(cx);
        let empty = composer.is_empty();
        let items = vec![
            ContextMenuItem::action("Cut", Cut).disabled(!request.has_selection),
            ContextMenuItem::action("Copy", Copy).disabled(!request.has_selection),
            ContextMenuItem::action("Paste", Paste),
            ContextMenuItem::separator(),
            ContextMenuItem::action("Select All", SelectAll).disabled(empty),
        ];
        let position = request.position;
        let from_keyboard = request.from_keyboard;
        // After this update: the composer is in the middle of emitting.
        window.defer(cx, move |window, cx| {
            let menu = ContextMenu::open_for(&target, position, items, window, cx);
            if from_keyboard {
                menu.update(cx, |menu, cx| menu.select_first(cx));
            }
        });
    }
}

/// Kept by the panel while its menu is open.
pub(crate) type MenuSubscription = Option<Subscription>;
