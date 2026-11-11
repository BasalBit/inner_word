//! The panel's action handlers (one per action in the key map) and its mouse handlers.

use gpui::{Context, ScrollStrategy, Window};

use super::rows::Row;
use super::{
    Collapse, CollapseAll, DeleteForever, Dismiss, Duplicate, EmptyTrash, Expand, ExpandAll,
    FocusFilter, FocusTree, LibraryItem, LibraryPanel, LibraryPanelEvent, MoveDown, MoveToFolder,
    MoveToTrash, MoveUp, NewDocument, NewFolder, OpenSelected, ROW_HEIGHT, Rename, RestoreSelected,
    RowDetail, SelectFirst, SelectLast, SelectNext, SelectPageDown, SelectPageUp, SelectPrevious,
    ShowTrash, TRASH_ROW_HEIGHT, ToggleRowDetail, UndoTrash, View,
};

impl LibraryPanel {
    pub(super) fn on_select_next(
        &mut self,
        _: &SelectNext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.view {
            View::Trash => {
                let next = self.selected_trash_index().map_or(0, |ix| ix + 1);
                self.select_trash_index(next, ScrollStrategy::Bottom, cx);
            }
            View::Tree if self.on_trash_row => {}
            // From the "Library" row down into the tree.
            View::Tree if self.on_root_row => {
                if self.rows.is_empty() {
                    self.select_trash_row(cx);
                } else {
                    self.select_index(0, ScrollStrategy::Top, cx);
                }
            }
            View::Tree => match self.selected_index() {
                Some(ix) if ix + 1 < self.rows.len() => {
                    self.select_index(ix + 1, ScrollStrategy::Bottom, cx)
                }
                // Past the last row comes the trash row, unless the user is typing a filter.
                Some(_) => {
                    if self.focus_handle.is_focused(window) {
                        self.select_trash_row(cx);
                    }
                }
                None if self.rows.is_empty() => {
                    if self.focus_handle.is_focused(window) {
                        self.select_trash_row(cx);
                    }
                }
                None => self.select_index(0, ScrollStrategy::Top, cx),
            },
        }
    }

    pub(super) fn on_select_previous(
        &mut self,
        _: &SelectPrevious,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.view {
            View::Trash => {
                let previous = match self.selected_trash_index() {
                    Some(ix) => ix.saturating_sub(1),
                    None => self.trash_rows.len().saturating_sub(1),
                };
                self.select_trash_index(previous, ScrollStrategy::Top, cx);
            }
            View::Tree if self.on_root_row => {}
            View::Tree => {
                // Above the first row is the "Library" row, unless the user is typing a
                // filter (the arrows then stay among the matches).
                let to_root = match self.selected_index() {
                    Some(ix) => ix == 0,
                    None => self.rows.is_empty(),
                };
                if to_root {
                    if self.focus_handle.is_focused(window) {
                        self.select_root(cx);
                    }
                    return;
                }
                let previous = match self.selected_index() {
                    Some(ix) => ix - 1,
                    None => self.rows.len() - 1,
                };
                self.select_index(previous, ScrollStrategy::Top, cx);
            }
        }
    }

    pub(super) fn on_select_first(
        &mut self,
        _: &SelectFirst,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.view {
            View::Trash => self.select_trash_index(0, ScrollStrategy::Top, cx),
            View::Tree => self.select_index(0, ScrollStrategy::Top, cx),
        }
    }

    pub(super) fn on_select_last(
        &mut self,
        _: &SelectLast,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.view {
            View::Trash => self.select_trash_index(
                self.trash_rows.len().saturating_sub(1),
                ScrollStrategy::Bottom,
                cx,
            ),
            View::Tree => self.select_index(
                self.rows.len().saturating_sub(1),
                ScrollStrategy::Bottom,
                cx,
            ),
        }
    }

    pub(super) fn on_select_page_up(
        &mut self,
        _: &SelectPageUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_by_page(false, cx);
    }

    pub(super) fn on_select_page_down(
        &mut self,
        _: &SelectPageDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_by_page(true, cx);
    }

    /// Moves the selection by one screenful less a row, so one row of context stays.
    pub(super) fn select_by_page(&mut self, down: bool, cx: &mut Context<Self>) {
        let (scroll, row_height, count, current) = match self.view {
            View::Tree => (
                &self.scroll,
                ROW_HEIGHT,
                self.rows.len(),
                self.selected_index(),
            ),
            View::Trash => (
                &self.trash_scroll,
                TRASH_ROW_HEIGHT,
                self.trash_rows.len(),
                self.selected_trash_index(),
            ),
        };
        if count == 0 {
            return;
        }
        // The list's height as of its last layout; before that, a guess.
        let page = scroll
            .0
            .borrow()
            .last_item_size
            .map(|size| (size.item.height / row_height).floor() as usize)
            .unwrap_or(10)
            .saturating_sub(1)
            .max(1);
        let target = match (current, down) {
            (Some(ix), true) => (ix + page).min(count - 1),
            (Some(ix), false) => ix.saturating_sub(page),
            (None, true) => 0,
            (None, false) => count - 1,
        };
        let strategy = if down {
            ScrollStrategy::Bottom
        } else {
            ScrollStrategy::Top
        };
        match self.view {
            View::Tree => self.select_index(target, strategy, cx),
            View::Trash => self.select_trash_index(target, strategy, cx),
        }
    }

    pub(super) fn on_expand(&mut self, _: &Expand, window: &mut Window, cx: &mut Context<Self>) {
        if self.view != View::Tree {
            return;
        }
        if self.on_trash_row {
            self.show_trash(window, cx);
            return;
        }
        if self.on_root_row {
            // Into the library: its first row.
            self.select_index(0, ScrollStrategy::Top, cx);
            return;
        }
        let Some(ix) = self.selected_index() else {
            return;
        };
        if let LibraryItem::Folder(id) = self.rows[ix].item {
            if !self.rows[ix].expanded {
                self.set_expanded(id, true, cx);
            } else if self
                .rows
                .get(ix + 1)
                .is_some_and(|row| row.parent == Some(id))
            {
                self.select_index(ix + 1, ScrollStrategy::Bottom, cx);
            }
        }
    }

    pub(super) fn on_collapse(
        &mut self,
        _: &Collapse,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.view == View::Trash {
            self.show_tree(cx);
            return;
        }
        let Some(row) = self.selected_row().cloned() else {
            return;
        };
        match row.item {
            // Under a filter every folder is drawn open, so there is nothing to collapse.
            LibraryItem::Folder(id) if row.expanded && self.filter.is_none() => {
                self.set_expanded(id, false, cx)
            }
            _ => match row.parent {
                Some(parent) => {
                    self.set_selected(Some(LibraryItem::Folder(parent)), cx);
                    self.scroll_to_selected(ScrollStrategy::Top);
                }
                // The parent of a top-level item is the library itself.
                None => {
                    if self.focus_handle.is_focused(window) {
                        self.select_root(cx);
                    }
                }
            },
        }
    }

    pub(super) fn on_open_selected(
        &mut self,
        _: &OpenSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.view == View::Trash {
            self.restore_selected(cx);
            return;
        }
        if self.on_trash_row {
            self.show_trash(window, cx);
            return;
        }
        match self.selected_row().map(|row| (row.item, row.expanded)) {
            Some((LibraryItem::Document(id), _)) => cx.emit(LibraryPanelEvent::OpenDocument {
                id,
                focus_editor: true,
            }),
            Some((LibraryItem::Folder(id), expanded)) if self.filter.is_none() => {
                self.set_expanded(id, !expanded, cx)
            }
            _ => {}
        }
    }

    pub(super) fn on_dismiss(&mut self, _: &Dismiss, _window: &mut Window, cx: &mut Context<Self>) {
        if self.renaming.is_some() {
            self.cancel_rename(cx);
        } else if self.view == View::Trash {
            self.show_tree(cx);
        } else if !self.filter_text.is_empty() {
            self.clear_filter(cx);
        } else {
            cx.emit(LibraryPanelEvent::Dismissed);
        }
    }

    pub(super) fn on_focus_tree(
        &mut self,
        _: &FocusTree,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.focus_handle.is_focused(window) {
            cx.propagate();
            return;
        }
        if self.renaming.is_some() {
            self.finish_rename(true, cx);
        }
        window.focus(&self.focus_handle);
    }

    pub(super) fn on_new_document(
        &mut self,
        _: &NewDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.new_document_content.is_none() {
            // The app creates documents itself: let its own handler have the action.
            cx.propagate();
            return;
        }
        self.new_document(window, cx);
    }

    pub(super) fn on_new_folder(
        &mut self,
        _: &NewFolder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_folder(window, cx);
    }

    /// Points a command for one item at the right item. With the keyboard in the panel
    /// that is the selected row. From anywhere else (a palette command or a menu-bar item
    /// while the keyboard is in the text) it is the document open in the editor: its row
    /// is selected first, so the sidebar shows what the command acted on. Without an open
    /// document the selection stays.
    pub(super) fn aim_at_open_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.view != View::Tree || self.focus_handle.contains_focused(window, cx) {
            return;
        }
        if let Some(id) = self.active_document
            && self.selected != Some(LibraryItem::Document(id))
        {
            self.reveal(LibraryItem::Document(id), cx);
        }
    }

    pub(super) fn on_rename(&mut self, _: &Rename, window: &mut Window, cx: &mut Context<Self>) {
        self.aim_at_open_document(window, cx);
        if let Some(item) = self.selected {
            self.begin_rename(item, window, cx);
        }
    }

    pub(super) fn on_duplicate(
        &mut self,
        _: &Duplicate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.aim_at_open_document(window, cx);
        self.duplicate_selected(cx);
    }

    pub(super) fn on_move_to_folder(
        &mut self,
        _: &MoveToFolder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.aim_at_open_document(window, cx);
        self.open_move_picker(window, cx);
    }

    pub(super) fn on_move_up(&mut self, _: &MoveUp, window: &mut Window, cx: &mut Context<Self>) {
        self.aim_at_open_document(window, cx);
        self.reorder_selected(-1, cx);
    }

    pub(super) fn on_move_down(
        &mut self,
        _: &MoveDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.aim_at_open_document(window, cx);
        self.reorder_selected(1, cx);
    }

    pub(super) fn on_move_to_trash(
        &mut self,
        _: &MoveToTrash,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.aim_at_open_document(window, cx);
        // The toast names the key that undoes: say where that key works when the
        // keyboard is somewhere else.
        self.keyboard_inside = self.focus_handle.contains_focused(window, cx);
        self.trash_selected(cx);
    }

    pub(super) fn on_undo_trash(
        &mut self,
        _: &UndoTrash,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.undo_trash(cx);
    }

    pub(super) fn on_focus_filter(
        &mut self,
        _: &FocusFilter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_filter(window, cx);
    }

    pub(super) fn on_show_trash(
        &mut self,
        _: &ShowTrash,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_trash(window, cx);
    }

    pub(super) fn on_restore_selected(
        &mut self,
        _: &RestoreSelected,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.restore_selected(cx);
    }

    pub(super) fn on_delete_forever(
        &mut self,
        _: &DeleteForever,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_selected_forever(window, cx);
    }

    pub(super) fn on_empty_trash(
        &mut self,
        _: &EmptyTrash,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.empty_trash(window, cx);
    }

    pub(super) fn on_toggle_row_detail(
        &mut self,
        _: &ToggleRowDetail,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_row_detail(
            match self.detail {
                RowDetail::Edited => RowDetail::Words,
                RowDetail::Words => RowDetail::Edited,
            },
            cx,
        );
    }

    // ----- mouse ----------------------------------------------------------------------

    pub(super) fn click_row(
        &mut self,
        ix: usize,
        clicks: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self.rows.get(ix).cloned() else {
            return;
        };
        if self.renaming == Some(row.item) {
            return;
        }
        if clicks >= 2 {
            // The first click of the pair opened or closed the folder: put it back, the
            // double click is about the name.
            if let Some(folder) = self.click_toggled.take()
                && row.item == LibraryItem::Folder(folder)
            {
                self.set_expanded(folder, !row.expanded, cx);
            }
            self.begin_rename(row.item, window, cx);
            return;
        }
        if self.renaming.is_some() {
            self.finish_rename(true, cx);
        }
        window.focus(&self.focus_handle);
        self.set_selected(Some(row.item), cx);
        self.click_toggled = None;
        match row.item {
            LibraryItem::Document(id) => cx.emit(LibraryPanelEvent::OpenDocument {
                id,
                focus_editor: false,
            }),
            // Under a filter every folder is drawn open: there is nothing to toggle.
            LibraryItem::Folder(id) if self.filter.is_none() => {
                self.set_expanded(id, !row.expanded, cx);
                self.click_toggled = Some(id);
            }
            LibraryItem::Folder(_) => {}
        }
    }

    /// A press of the right button on the "Trash" row under the tree.
    pub(super) fn click_trash_footer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.renaming.is_some() {
            self.finish_rename(true, cx);
        }
        window.focus(&self.focus_handle);
        self.select_trash_row(cx);
    }

    /// Runs `action` for a mouse control of the panel (a button, a menu item) the way its
    /// key would: the panel takes the keyboard if it does not have it, and the action is
    /// dispatched there, so it reaches the same handler as the key (and the app's own
    /// handler where the panel passes an action on).
    pub(super) fn run_from_mouse(
        &mut self,
        action: Box<dyn gpui::Action>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.focus_handle.contains_focused(window, cx) {
            window.focus(&self.focus_handle);
        }
        let handle = self.focus_handle.clone();
        // Dispatched at the list itself, not at a text field of the panel that may have
        // the keyboard (the filter): the keys of the list do not apply there.
        window.defer(cx, move |window, cx| {
            if !handle.is_focused(window) {
                window.focus(&handle);
            }
            handle.dispatch_action(action.as_ref(), window, cx);
        });
    }

    pub(super) fn on_expand_all(
        &mut self,
        _: &ExpandAll,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_expanded_below_selection(true, cx);
    }

    pub(super) fn on_collapse_all(
        &mut self,
        _: &CollapseAll,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_expanded_below_selection(false, cx);
    }

    /// Expands or collapses the selected folder and every folder inside it; with the
    /// "Library" row selected, every folder of the library.
    fn set_expanded_below_selection(&mut self, expanded: bool, cx: &mut Context<Self>) {
        if self.view != View::Tree || self.filter.is_some() {
            return;
        }
        match self.selected {
            Some(LibraryItem::Folder(folder)) => self.set_expanded_all(Some(folder), expanded, cx),
            None if self.on_root_row => self.set_expanded_all(None, expanded, cx),
            _ => {}
        }
    }

    /// A click on the "Library" row above the tree.
    pub(super) fn click_root_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.renaming.is_some() {
            self.finish_rename(true, cx);
        }
        window.focus(&self.focus_handle);
        self.select_root(cx);
    }

    pub(super) fn click_disclosure(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.filter.is_some() {
            return;
        }
        if let Some(Row {
            item: LibraryItem::Folder(id),
            expanded,
            ..
        }) = self.rows.get(ix)
        {
            self.set_expanded(*id, !*expanded, cx);
        }
    }

    pub(super) fn click_trash_row(
        &mut self,
        ix: usize,
        clicks: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);
        self.select_trash_index(ix, ScrollStrategy::Center, cx);
        if clicks >= 2 {
            self.restore_selected(cx);
        }
    }
}
