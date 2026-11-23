//! The context menus of the panel: what a right click (or `ctrl-enter`, or a "more"
//! button) offers on a row, on the library itself, and in the trash.
//!
//! Every item is one of the panel's actions. Choosing it dispatches that action at the
//! list, so it does exactly what its key does, and the menu shows that key.

use gpui::{Action, App, Context, Pixels, Point, SharedString, Window, point, px};
use innerword_store::{DocumentId, FolderId, FolderNode};
use innerword_ui::context_menu::OpenContextMenu;
use innerword_ui::{ContextMenu, ContextMenuItem, MenuAnchor};

use super::{
    CollapseAll, DeleteForever, Dismiss, Duplicate, EmptyTrash, ExpandAll, LibraryItem,
    LibraryPanel, MoveToFolder, MoveToTrash, NewBoard, NewDocument, NewFolder, OpenSelected,
    ROW_HEIGHT, Rename, RestoreSelected, ShowTrash, TRASH_ROW_HEIGHT, View,
};

/// What the app adds to a menu of the panel: see [`LibraryPanel::set_menu_extension`].
pub(super) type MenuExtension =
    std::rc::Rc<dyn Fn(LibraryMenuTarget, &mut Window, &mut App) -> Vec<ContextMenuItem>>;

/// What a context menu of the panel was opened on. The app receives it in its
/// [menu extension](LibraryPanel::set_menu_extension) and decides what to add.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryMenuTarget {
    /// A document row of the tree.
    Document(DocumentId),
    /// A folder row of the tree.
    Folder(FolderId),
    /// The library itself: the "Library" row, or the empty area under the rows.
    Library,
    /// An item in the trash view.
    Trashed(LibraryItem),
    /// The trash as a whole: the "Trash" row under the tree, or the empty area of the
    /// trash view.
    Trash,
}

/// Which menu to open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MenuKind {
    /// On a row of the tree.
    Row(LibraryItem),
    /// On the "Library" row or the empty area under the rows.
    Library,
    /// From the "+" button of the header.
    Add,
    /// On the "Trash" row under the tree.
    TrashRow,
    /// On an item of the trash view.
    TrashItem(LibraryItem),
    /// On the empty area of the trash view.
    TrashBackground,
}

impl MenuKind {
    /// What the app is told the menu is on (`None`: a menu the app adds nothing to).
    pub(super) fn target(self) -> Option<LibraryMenuTarget> {
        match self {
            MenuKind::Row(LibraryItem::Document(id)) => Some(LibraryMenuTarget::Document(id)),
            MenuKind::Row(LibraryItem::Folder(id)) => Some(LibraryMenuTarget::Folder(id)),
            MenuKind::Library => Some(LibraryMenuTarget::Library),
            MenuKind::Add => None,
            MenuKind::TrashRow | MenuKind::TrashBackground => Some(LibraryMenuTarget::Trash),
            MenuKind::TrashItem(item) => Some(LibraryMenuTarget::Trashed(item)),
        }
    }
}

/// One line of a menu of the panel, before it is handed to the menu widget.
pub(super) enum Entry {
    /// An action of the panel.
    Action {
        label: SharedString,
        action: Box<dyn Action>,
        enabled: bool,
        /// Destroys something or is hard to take back: drawn as such.
        danger: bool,
    },
    /// A line between groups.
    Separator,
    /// Where the app's own items for this target go.
    Extension,
}

fn entry(label: &'static str, action: impl Action) -> Entry {
    Entry::Action {
        label: label.into(),
        action: Box::new(action),
        enabled: true,
        danger: false,
    }
}

impl Entry {
    fn enabled(mut self, on: bool) -> Self {
        if let Entry::Action { enabled, .. } = &mut self {
            *enabled = on;
        }
        self
    }

    fn danger(mut self) -> Self {
        if let Entry::Action { danger, .. } = &mut self {
            *danger = true;
        }
        self
    }
}

/// Whether `node` or a folder inside it hides something: it has contents and is collapsed.
fn any_collapsed(node: &FolderNode, expanded: &std::collections::HashSet<FolderId>) -> bool {
    let has_children = !node.folders.is_empty() || !node.documents.is_empty();
    (has_children && !expanded.contains(&node.folder.id))
        || node
            .folders
            .iter()
            .any(|child| any_collapsed(child, expanded))
}

impl LibraryPanel {
    /// Lets the app add its own items to the panel's context menus (the panel cannot know
    /// about exporting or version history). `extension` is called every time a menu is
    /// about to open, with what the menu is on, and returns the items to add; an empty
    /// list adds nothing. They are placed after the panel's own items for the target and
    /// before the destructive one ("Move to Trash"), with a separator when needed.
    ///
    /// The row that was clicked is selected before the menu opens, so an item may act on
    /// the target it was given or on [`selected`](Self::selected).
    ///
    /// ```ignore
    /// panel.update(cx, |panel, _| {
    ///     panel.set_menu_extension(move |target, _window, _cx| match target {
    ///         LibraryMenuTarget::Document(id) => vec![
    ///             ContextMenuItem::action("Export…", ExportDocument { id }),
    ///             ContextMenuItem::new("Version History", move |window, cx| { /* open it for id */ }),
    ///         ],
    ///         LibraryMenuTarget::Folder(id) => vec![ContextMenuItem::action("Export…", ExportFolder { id })],
    ///         _ => Vec::new(),
    ///     });
    /// });
    /// ```
    ///
    /// An action item is dispatched at the panel's list, so it travels from the panel up
    /// to the window root like one of the panel's own keys.
    pub fn set_menu_extension(
        &mut self,
        extension: impl Fn(LibraryMenuTarget, &mut Window, &mut App) -> Vec<ContextMenuItem> + 'static,
    ) {
        self.menu_extension = Some(std::rc::Rc::new(extension));
    }

    /// Opens the menu `kind` at `anchor` (a window position, or the bounds it drops down
    /// from). Its action items are dispatched at the list of the panel, whatever has the
    /// keyboard, so they do what their keys do there.
    pub(super) fn open_menu(
        &mut self,
        kind: MenuKind,
        anchor: impl Into<MenuAnchor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let extra = match (kind.target(), self.menu_extension.clone()) {
            (Some(target), Some(extension)) => extension(target, window, cx),
            _ => Vec::new(),
        };
        let mut items = Vec::new();
        for entry in self.menu_entries(kind, cx) {
            match entry {
                Entry::Action {
                    label,
                    action,
                    enabled,
                    danger,
                } => {
                    let mut item = ContextMenuItem::action_boxed(label, action).disabled(!enabled);
                    if danger {
                        item = item.danger();
                    }
                    items.push(item);
                }
                Entry::Separator => items.push(ContextMenuItem::separator()),
                Entry::Extension => {
                    if !extra.is_empty() {
                        items.push(ContextMenuItem::separator());
                        items.extend(extra.iter().cloned());
                    }
                }
            }
        }
        let menu = ContextMenu::open_for(&self.focus_handle, anchor, items, window, cx);
        if self.menu_from_keyboard {
            // Opened with the key: start on the first item, so `enter` chooses it.
            menu.update(cx, |menu, cx| menu.select_first(cx));
        }
        self.menu_from_keyboard = false;
    }

    /// The titles of the context menu that is open in the panel's window, top to bottom,
    /// with "-" for a separator; `None` when no menu is open. For tests and automation.
    pub fn open_menu_labels(&self, window: &Window, cx: &App) -> Option<Vec<String>> {
        ContextMenu::current(window, cx).map(|menu| menu.read(cx).labels())
    }

    /// The lines of the menu `kind`, top to bottom.
    pub(super) fn menu_entries(&self, kind: MenuKind, cx: &Context<Self>) -> Vec<Entry> {
        let filtering = self.filter.is_some();
        match kind {
            MenuKind::Row(LibraryItem::Document(_)) => vec![
                entry("Open", OpenSelected),
                Entry::Separator,
                entry("Rename", Rename),
                entry("Duplicate", Duplicate),
                entry("Move To…", MoveToFolder),
                Entry::Extension,
                Entry::Separator,
                entry("Move to Trash", MoveToTrash).danger(),
            ],
            MenuKind::Row(LibraryItem::Folder(folder)) => {
                let tree = self.library.read(cx).tree();
                let node = tree.find_folder(folder);
                let has_children =
                    node.is_some_and(|node| !node.folders.is_empty() || !node.documents.is_empty());
                // One line that says what a press would do now.
                let expand = node.is_none_or(|node| any_collapsed(node, &self.expanded));
                let all = if expand || !has_children {
                    entry("Expand All", ExpandAll)
                } else {
                    entry("Collapse All", CollapseAll)
                };
                let mut entries = vec![
                    entry("New Document Here", NewDocument),
                    entry("New Folder Here", NewFolder),
                ];
                if self.makes_boards() {
                    entries.push(entry("New Whiteboard Here", NewBoard));
                }
                entries.extend([
                    Entry::Separator,
                    entry("Rename", Rename),
                    entry("Duplicate", Duplicate),
                    entry("Move To…", MoveToFolder),
                    all.enabled(has_children && !filtering),
                    Entry::Extension,
                    Entry::Separator,
                    entry("Move to Trash", MoveToTrash).danger(),
                ]);
                entries
            }
            MenuKind::Library => {
                let tree = self.library.read(cx).tree();
                let expand = tree
                    .folders
                    .iter()
                    .any(|node| any_collapsed(node, &self.expanded));
                let all = if expand || tree.folders.is_empty() {
                    entry("Expand All", ExpandAll)
                } else {
                    entry("Collapse All", CollapseAll)
                };
                let mut entries = vec![
                    entry("New Document", NewDocument),
                    entry("New Folder", NewFolder),
                ];
                if self.makes_boards() {
                    entries.push(entry("New Whiteboard", NewBoard));
                }
                entries.extend([
                    Entry::Separator,
                    all.enabled(!tree.folders.is_empty() && !filtering),
                    Entry::Extension,
                    Entry::Separator,
                    entry("Show Trash", ShowTrash),
                ]);
                entries
            }
            MenuKind::Add => {
                let mut entries = vec![
                    entry("New Document", NewDocument),
                    entry("New Folder", NewFolder),
                ];
                if self.makes_boards() {
                    entries.push(entry("New Whiteboard", NewBoard));
                }
                entries
            }
            MenuKind::TrashRow => vec![
                entry("Show Trash", ShowTrash),
                Entry::Extension,
                Entry::Separator,
                entry("Empty Trash…", EmptyTrash)
                    .enabled(!self.trash_rows.is_empty())
                    .danger(),
            ],
            MenuKind::TrashItem(_) => vec![
                entry("Restore", RestoreSelected),
                Entry::Extension,
                Entry::Separator,
                entry("Delete Forever…", DeleteForever).danger(),
            ],
            MenuKind::TrashBackground => vec![
                entry("Back to Library", Dismiss),
                Entry::Extension,
                Entry::Separator,
                entry("Empty Trash…", EmptyTrash)
                    .enabled(!self.trash_rows.is_empty())
                    .danger(),
            ],
        }
    }

    /// Opens the menu of tree row `ix` at `position`, after selecting that row (the menu
    /// acts on the selection, as the keys do). The panel takes the keyboard.
    pub(super) fn open_row_menu(
        &mut self,
        ix: usize,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.rows.get(ix).map(|row| row.item) else {
            return;
        };
        // The row whose name is being typed: the click belongs to its text field.
        if self.renaming == Some(item) {
            return;
        }
        if self.renaming.is_some() {
            self.finish_rename(true, cx);
        }
        window.focus(&self.focus_handle);
        self.set_selected(Some(item), cx);
        self.open_menu(MenuKind::Row(item), position, window, cx);
    }

    /// Opens the menu of row `ix` of the trash view at `position`, after selecting it.
    pub(super) fn open_trash_item_menu(
        &mut self,
        ix: usize,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.trash_rows.get(ix).map(|row| row.item) else {
            return;
        };
        window.focus(&self.focus_handle);
        self.trash_selected = Some(item);
        cx.notify();
        self.open_menu(MenuKind::TrashItem(item), position, window, cx);
    }

    /// Opens the menu of the "+" button, under the button. New things go where the keys
    /// `n` and `shift-n` put them: into the selected folder, or to the top level.
    pub(super) fn open_add_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Something new goes where the selection is: the row that was selected before a
        // filter hid it is not a place the user can see, the list itself is.
        if !self.focus_handle.contains_focused(window, cx) {
            window.focus(&self.focus_handle);
        }
        self.open_menu(MenuKind::Add, self.anchors.add_button.get(), window, cx);
    }

    /// `ctrl-enter`: the menu of what is selected, opened at its row.
    pub(super) fn on_open_context_menu(
        &mut self,
        _: &OpenContextMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.renaming.is_some() {
            return;
        }
        self.menu_from_keyboard = true;
        // A little in from the left edge and just under the row, so the row stays readable.
        let under = |bounds: gpui::Bounds<Pixels>| point(bounds.left() + px(24.), bounds.bottom());
        match self.view {
            View::Trash => match self.selected_trash_index() {
                Some(ix) => {
                    let handle = self.trash_scroll.0.borrow().base_handle.clone();
                    let list = handle.bounds();
                    let top = list.top() + handle.offset().y + TRASH_ROW_HEIGHT * ix as f32;
                    let position = point(list.left() + px(24.), top + TRASH_ROW_HEIGHT);
                    self.open_trash_item_menu(ix, position, window, cx);
                }
                None => {
                    let list = self.trash_scroll.0.borrow().base_handle.bounds();
                    let position = point(list.left() + px(24.), list.top() + px(8.));
                    self.open_menu(MenuKind::TrashBackground, position, window, cx);
                }
            },
            View::Tree if self.on_trash_row => {
                let row = self.anchors.trash_row.get();
                // Above the row: it is at the bottom of the panel.
                self.open_menu(
                    MenuKind::TrashRow,
                    point(row.left() + px(24.), row.top()),
                    window,
                    cx,
                );
            }
            View::Tree => match self.selected_index() {
                Some(ix) => {
                    self.scroll.scroll_to_item(ix, gpui::ScrollStrategy::Center);
                    let mut bounds = self.row_bounds(ix);
                    // A row that is scrolled out of view: open at the edge of the list.
                    let list = self.scroll.0.borrow().base_handle.bounds();
                    bounds.origin.y = bounds
                        .origin
                        .y
                        .clamp(list.top() - ROW_HEIGHT, list.bottom() - ROW_HEIGHT);
                    self.open_row_menu(ix, under(bounds), window, cx);
                }
                None => {
                    self.select_root(cx);
                    let position = under(self.anchors.root_row.get());
                    self.open_menu(MenuKind::Library, position, window, cx);
                }
            },
        }
    }
}
