//! Drag and drop in the tree: a row dragged onto a folder moves into it, dragged between
//! two rows it takes that place among its siblings, and dragged onto the empty area or the
//! "Library" row it moves to the top level.
//!
//! The moves go through the same [`Library`](crate::Library) calls as the keyboard paths
//! (`m`, `alt-up` / `alt-down`) and are recorded for `cmd-z` in the same way.

use std::path::PathBuf;
use std::time::Duration;

use gpui::{Context, IntoElement, Pixels, Render, SharedString, Window, div, prelude::*, px};
use writer_store::FolderId;
use writer_ui::theme::{ActiveTheme, RADIUS_SM, SPACE_2, UI_TEXT_BASE};
use writer_ui::toast::show_toast;
use writer_ui::{Icon, IconName, Toast};

use super::rows::{ancestors, place_of};
use super::{LibraryItem, LibraryPanel, LibraryPanelEvent, ROW_HEIGHT, View};

/// How long a drag rests on a collapsed folder before the folder opens.
pub(super) const EXPAND_ON_HOVER: Duration = Duration::from_millis(650);
/// How close to the top or bottom edge of the list a drag scrolls it.
const SCROLL_EDGE: Pixels = px(18.);

/// The icon of an item of the library, wherever the panel draws one (a row of the tree, a
/// row of the trash, the ghost of a drag): a folder (open or closed), a text, or a page,
/// which has the globe.
pub(super) fn item_icon(item: LibraryItem, page: bool, open: bool) -> IconName {
    match item {
        LibraryItem::Folder(_) if open => IconName::FolderOpen,
        LibraryItem::Folder(_) => IconName::Folder,
        LibraryItem::Document(_) if page => IconName::Globe,
        LibraryItem::Document(_) => IconName::Document,
    }
}

/// What a drag out of the tree carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DraggedItem {
    pub item: LibraryItem,
    pub label: SharedString,
    /// The item is a page: the ghost shows the globe.
    pub page: bool,
}

/// What follows the pointer during a drag: the icon and name of the dragged item, a little
/// below and to the right of the pointer, so the row under the pointer stays readable.
pub(super) struct DragPreview {
    dragged: DraggedItem,
    /// Where in its row the pointer was when the drag began. GPUI draws this view that
    /// far up and left of the pointer.
    grabbed_at: gpui::Point<Pixels>,
}

impl DragPreview {
    pub fn new(dragged: DraggedItem, grabbed_at: gpui::Point<Pixels>) -> Self {
        Self {
            dragged,
            grabbed_at,
        }
    }
}

impl Render for DragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = theme.colors;
        let label = div()
            .flex()
            .items_center()
            .gap(SPACE_2)
            .h(ROW_HEIGHT)
            .max_w(px(260.))
            .px(SPACE_2)
            .rounded(RADIUS_SM)
            .border_1()
            .border_color(colors.border_strong)
            .bg(colors.elevated)
            .shadow(theme.shadow_popover())
            .font_family(theme.fonts.ui.clone())
            .text_size(UI_TEXT_BASE)
            .text_color(colors.ink)
            .child(
                Icon::new(item_icon(self.dragged.item, self.dragged.page, false))
                    .color(colors.muted),
            )
            .child(div().min_w_0().truncate().child(self.dragged.label.clone()));
        div()
            .pl(self.grabbed_at.x + px(14.))
            .pt(self.grabbed_at.y + px(12.))
            .child(label)
    }
}

/// Where a dragged item would land if it were dropped now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DropTarget {
    /// Into this folder, at the end (`None` is the top level of the library).
    Into(Option<FolderId>),
    /// Between two rows.
    Between {
        /// The folder whose contents the gap is in.
        parent: Option<FolderId>,
        /// The place among the siblings of the dragged kind, counted with the dragged
        /// item still where it is.
        index: usize,
        /// The row the line is drawn on.
        row: usize,
        /// The line is at the bottom edge of that row (otherwise at its top edge).
        below: bool,
    },
    /// Onto the "Trash" row under the tree: to the trash.
    Trash,
}

impl LibraryPanel {
    /// Where `dragged` would land when dropped on row `ix`, with the pointer `fraction`
    /// of the way down the row. `None` when a drop there would change nothing or is not
    /// allowed: on itself, a folder into itself or into one of its own subfolders, an
    /// item onto the folder it is already in.
    pub(super) fn drop_target_on_row(
        &self,
        dragged: LibraryItem,
        ix: usize,
        fraction: f32,
        cx: &Context<Self>,
    ) -> Option<DropTarget> {
        let row = self.rows.get(ix)?;
        if row.item == dragged || self.view != View::Tree {
            return None;
        }
        let tree = self.library.read(cx).tree();
        let (from_parent, from_index) = place_of(&tree, dragged)?;
        // A folder cannot go into itself or below itself.
        let inside_dragged = |target: Option<FolderId>| match (dragged, target) {
            (LibraryItem::Folder(folder), Some(target)) => {
                target == folder
                    || ancestors(&tree, LibraryItem::Folder(target))
                        .is_none_or(|path| path.contains(&folder))
            }
            _ => false,
        };
        let into = |folder: Option<FolderId>| {
            (from_parent != folder && !inside_dragged(folder)).then_some(DropTarget::Into(folder))
        };
        // Under a filter the siblings are not all on screen, so a place between two rows
        // would not be the place it looks like: only moves into folders are offered.
        let filtering = self.filter.is_some();
        match (dragged, row.item) {
            (LibraryItem::Document(_), LibraryItem::Folder(folder)) => into(Some(folder)),
            // Folders come before documents: a folder cannot sit among documents, so
            // over a document it goes to that document's folder.
            (LibraryItem::Folder(_), LibraryItem::Document(_)) => into(row.parent),
            (LibraryItem::Folder(_), LibraryItem::Folder(folder)) => {
                // The lower edge of an open folder is inside it, not after it.
                let open = row.expanded && row.has_children;
                let before = fraction < 0.25;
                let after = fraction > 0.75 && !open;
                if filtering || !(before || after) {
                    return into(Some(folder));
                }
                self.between(dragged, (from_parent, from_index), ix, after, cx)
            }
            (LibraryItem::Document(_), LibraryItem::Document(_)) => {
                if filtering {
                    return into(row.parent);
                }
                self.between(dragged, (from_parent, from_index), ix, fraction >= 0.5, cx)
            }
        }
    }

    /// The gap before or after row `ix`, which is of the same kind as `dragged`.
    fn between(
        &self,
        dragged: LibraryItem,
        from: (Option<FolderId>, usize),
        ix: usize,
        below: bool,
        cx: &Context<Self>,
    ) -> Option<DropTarget> {
        let row = self.rows.get(ix)?;
        let tree = self.library.read(cx).tree();
        let (parent, row_index) = place_of(&tree, row.item)?;
        if let (LibraryItem::Folder(folder), Some(parent)) = (dragged, parent)
            && (parent == folder
                || ancestors(&tree, LibraryItem::Folder(parent))
                    .is_none_or(|path| path.contains(&folder)))
        {
            return None;
        }
        let index = row_index + usize::from(below);
        // Right before or right after itself is where it already is.
        if parent == from.0 && (index == from.1 || index == from.1 + 1) {
            return None;
        }
        Some(DropTarget::Between {
            parent,
            index,
            row: ix,
            below,
        })
    }

    /// Where `dragged` would land when dropped on the "Library" row or on the empty area
    /// under the rows: the top level, unless it is there already.
    pub(super) fn drop_target_on_root(
        &self,
        dragged: LibraryItem,
        cx: &Context<Self>,
    ) -> Option<DropTarget> {
        if self.view != View::Tree {
            return None;
        }
        let (parent, _) = place_of(&self.library.read(cx).tree(), dragged)?;
        parent.is_some().then_some(DropTarget::Into(None))
    }

    /// Where the drag in progress would land if it were dropped now, in words: "into
    /// folder 3", "into the library", "between in folder 3 at 1", "trash". `None` when no
    /// drag is over a place it can land. For tests.
    #[doc(hidden)]
    pub fn drop_indicator(&self) -> Option<String> {
        let folder = |folder: Option<FolderId>| match folder {
            Some(id) => format!("folder {}", id.0),
            None => "the library".to_string(),
        };
        Some(match self.drop_target? {
            DropTarget::Into(target) => format!("into {}", folder(target)),
            DropTarget::Between { parent, index, .. } => {
                format!("between in {} at {index}", folder(parent))
            }
            DropTarget::Trash => "trash".to_string(),
        })
    }

    /// The place a drop would land changed (the pointer moved during a drag). A drag that
    /// rests on a collapsed folder opens it after a moment.
    pub(super) fn set_drop_target(&mut self, target: Option<DropTarget>, cx: &mut Context<Self>) {
        if self.drop_target == target {
            return;
        }
        self.drop_target = target;
        self.drop_expand = None;
        if let Some(DropTarget::Into(Some(folder))) = target
            && self.filter.is_none()
            && !self.expanded.contains(&folder)
        {
            self.drop_expand = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(EXPAND_ON_HOVER).await;
                this.update(cx, |this, cx| {
                    this.drop_expand = None;
                    if this.drop_target == Some(DropTarget::Into(Some(folder))) {
                        this.set_expanded(folder, true, cx);
                    }
                })
                .ok();
            }));
        }
        cx.notify();
    }

    /// The drag ended without a drop in the tree, or left it.
    pub(super) fn clear_drop_target(&mut self, cx: &mut Context<Self>) {
        self.set_drop_target(None, cx);
    }

    /// Scrolls the list when a drag comes close to its top or bottom edge, so a row can
    /// be dragged to a place that is not in view (one row for every move of the pointer).
    /// Returns whether it scrolled.
    pub(super) fn scroll_for_drag(&mut self, position: gpui::Point<Pixels>) -> bool {
        let handle = self.scroll.0.borrow().base_handle.clone();
        let bounds = handle.bounds();
        if !bounds.contains(&position) {
            return false;
        }
        let before = handle.offset();
        let mut offset = before;
        if position.y < bounds.top() + SCROLL_EDGE {
            offset.y = (offset.y + ROW_HEIGHT).min(px(0.));
        } else if position.y > bounds.bottom() - SCROLL_EDGE {
            offset.y = (offset.y - ROW_HEIGHT).max(-handle.max_offset().height);
        }
        if offset == before {
            return false;
        }
        handle.set_offset(offset);
        true
    }

    /// Where row `ix` of the tree is drawn, in window coordinates (as of the last frame).
    pub(super) fn row_bounds(&self, ix: usize) -> gpui::Bounds<Pixels> {
        let handle = self.scroll.0.borrow().base_handle.clone();
        let list = handle.bounds();
        gpui::Bounds {
            origin: gpui::point(
                list.left(),
                list.top() + handle.offset().y + ROW_HEIGHT * ix as f32,
            ),
            size: gpui::size(list.size.width, ROW_HEIGHT),
        }
    }

    /// The row of the tree at `position` (window coordinates), if there is one there.
    pub(super) fn row_at(&self, position: gpui::Point<Pixels>) -> Option<usize> {
        let handle = self.scroll.0.borrow().base_handle.clone();
        let list = handle.bounds();
        if self.rows.is_empty() || !list.contains(&position) {
            return None;
        }
        let y = position.y - list.top() - handle.offset().y;
        let ix = (y / ROW_HEIGHT).floor();
        (ix >= 0. && (ix as usize) < self.rows.len()).then_some(ix as usize)
    }

    /// The folder that files dropped at `position` (window coordinates) go into, as
    /// [`LibraryPanelEvent::DropPaths`] names it: the folder of a folder row; the folder a
    /// document row is in (`None` at the top level); the top level on the "Library" row;
    /// anywhere else on the panel (the empty area under the rows, the filter, the trash
    /// row, the trash view) the folder selected, else the top level.
    pub(super) fn folder_for_files_at(&self, position: gpui::Point<Pixels>) -> Option<FolderId> {
        if self.view == View::Tree {
            if let Some(row) = self.row_at(position).and_then(|ix| self.rows.get(ix)) {
                return match row.item {
                    LibraryItem::Folder(id) => Some(id),
                    LibraryItem::Document(_) => row.parent,
                };
            }
            if self.anchors.root_row.get().contains(&position) {
                return None;
            }
        }
        self.selected_folder()
    }

    /// Files from outside the app are dragged over the panel at `position`: the place they
    /// would go is marked as a dragged row's is (the folder's row, or the "Library" row),
    /// and a collapsed folder under them opens after a moment.
    pub(super) fn files_dragged_over(
        &mut self,
        position: gpui::Point<Pixels>,
        inside: bool,
        cx: &mut Context<Self>,
    ) {
        if !inside {
            self.clear_drop_target(cx);
            return;
        }
        let target = DropTarget::Into(self.folder_for_files_at(position));
        self.set_drop_target(Some(target), cx);
        // Only a folder under the pointer opens by itself, not the selected one the files
        // would go to from the empty area.
        let on_a_folder_row = self.view == View::Tree
            && self
                .row_at(position)
                .and_then(|ix| self.rows.get(ix))
                .is_some_and(|row| matches!(row.item, LibraryItem::Folder(_)));
        if !on_a_folder_row {
            self.drop_expand = None;
        }
    }

    /// Files from outside the app were dropped on the panel at `position` (window
    /// coordinates): emits [`LibraryPanelEvent::DropPaths`] with the folder under them
    /// ([`folder_for_files_at`](Self::folder_for_files_at)). The panel takes none of them
    /// itself; the app decides what to do with which. Nothing is emitted for no paths.
    ///
    /// This is what the panel's drop listener calls. It is public so that a headless test
    /// can drop real paths: GPUI's `ExternalPaths` cannot be built outside GPUI with
    /// paths in it. Not part of the API.
    #[doc(hidden)]
    pub fn files_dropped(
        &mut self,
        paths: Vec<PathBuf>,
        position: gpui::Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let folder = self.folder_for_files_at(position);
        self.drop_target = None;
        self.drop_expand = None;
        cx.notify();
        if paths.is_empty() {
            return;
        }
        cx.emit(LibraryPanelEvent::DropPaths { paths, folder });
    }

    /// The dragged item was dropped: move it to where the indicator was. The panel takes
    /// the keyboard, so `cmd-z` right after the drop takes it back.
    pub(super) fn finish_drop(
        &mut self,
        dragged: &DraggedItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.drop_target.take();
        self.drop_expand = None;
        cx.notify();
        let Some(target) = target else {
            return;
        };
        if self.renaming.is_some() {
            self.finish_rename(true, cx);
        }
        window.focus(&self.focus_handle);
        match target {
            DropTarget::Into(folder) => self.move_item_and_report(dragged.item, folder, cx),
            DropTarget::Between { parent, index, .. } => {
                self.place_item(dragged.item, parent, index, cx)
            }
            DropTarget::Trash => {
                self.keyboard_inside = true;
                self.trash_item(dragged.item, cx);
            }
        }
    }

    /// Puts `item` into `parent` at gap `index` among its siblings of the same kind (the
    /// gap is counted with the item still where it is). Within its own folder that is a
    /// reorder, otherwise a move; [`undo`](Self::undo) takes either back.
    pub(super) fn place_item(
        &mut self,
        item: LibraryItem,
        parent: Option<FolderId>,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(from) = place_of(&self.library.read(cx).tree(), item) else {
            return;
        };
        if from.0 == parent {
            let target = if from.1 < index { index - 1 } else { index };
            if target == from.1 {
                return;
            }
            let moved = self.library.update(cx, |library, cx| match item {
                LibraryItem::Folder(id) => library.reorder_folder(id, target, cx),
                LibraryItem::Document(id) => library.reorder_document(id, target, cx),
            });
            match moved {
                Ok(()) => {
                    self.remember_place(item, from, false);
                    self.reveal(item, cx);
                }
                Err(err) => self.report_error(&err.to_string(), cx),
            }
            return;
        }
        let name = self.name_of(item, cx);
        let moved = self.library.update(cx, |library, cx| match item {
            LibraryItem::Folder(id) => library.move_folder(id, parent, Some(index), cx),
            LibraryItem::Document(id) => library.move_document(id, parent, Some(index), cx),
        });
        match moved {
            Ok(()) => {
                self.remember_place(item, from, true);
                self.reveal(item, cx);
                let place = self.place_label(parent, cx);
                let toast = Toast::new(format!("Moved “{name}” to {place}."));
                show_toast(self.with_undo_button(toast, cx), cx);
            }
            Err(err) => self.report_error(&err.to_string(), cx),
        }
    }
}
