//! [`LibraryPanel`]: the library sidebar.
//!
//! **Status: skeleton.** The public API below is final. The rendering is a plain
//! indented list with keyboard navigation (move, expand, collapse, open). Inline rename,
//! create / trash / duplicate / move commands (also in [`commands`]), the trash section and
//! drag and drop are added by the panel implementation.

use std::collections::HashSet;

use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyBinding,
    ScrollHandle, SharedString, Subscription, Window, actions, div, prelude::*, px,
};
use writer_store::{DocumentId, FolderId, FolderNode, LibraryTree};
use writer_ui::theme::{ActiveTheme, RADIUS_SM, SPACE_1, SPACE_2, SPACE_4, UI_TEXT_BASE};
use writer_ui::{CommandDescriptor, Icon, IconName};

use crate::library::{Library, LibraryEvent};

actions!(
    library_panel,
    [
        /// Select the next visible row.
        SelectNext,
        /// Select the previous visible row.
        SelectPrevious,
        /// Select the first row.
        SelectFirst,
        /// Select the last visible row.
        SelectLast,
        /// Expand the selected folder, or step into it when it is already expanded.
        Expand,
        /// Collapse the selected folder, or step out to the parent folder.
        Collapse,
        /// Open the selected document in the editor, or toggle the selected folder.
        OpenSelected,
        /// Leave the panel (the app returns focus to the editor).
        Dismiss,
    ]
);

/// The key context of the panel's root element; use it in binding predicates.
pub const KEY_CONTEXT: &str = "LibraryPanel";

/// The settings key under which the panel stores the ids of the expanded folders
/// (a JSON array of folder ids).
pub const EXPANDED_FOLDERS_SETTING: &str = "library.expanded_folders";

/// The panel's default key bindings, all scoped to [`KEY_CONTEXT`]. Installed by
/// [`crate::init`].
pub fn key_bindings() -> Vec<KeyBinding> {
    let context = Some(KEY_CONTEXT);
    vec![
        KeyBinding::new("down", SelectNext, context),
        KeyBinding::new("up", SelectPrevious, context),
        KeyBinding::new("ctrl-n", SelectNext, context),
        KeyBinding::new("ctrl-p", SelectPrevious, context),
        KeyBinding::new("cmd-up", SelectFirst, context),
        KeyBinding::new("cmd-down", SelectLast, context),
        KeyBinding::new("home", SelectFirst, context),
        KeyBinding::new("end", SelectLast, context),
        KeyBinding::new("right", Expand, context),
        KeyBinding::new("left", Collapse, context),
        KeyBinding::new("enter", OpenSelected, context),
        KeyBinding::new("escape", Dismiss, context),
    ]
}

/// The panel's commands for the command palette and the shortcuts overlay. Their actions
/// are handled by the panel, so they act when the panel has focus.
///
/// Skeleton: only the navigation commands exist so far; the panel implementation adds
/// rename, trash, duplicate and move here.
pub fn commands() -> Vec<CommandDescriptor> {
    const CATEGORY: &str = "Library";
    vec![
        CommandDescriptor::new("Open selected item", CATEGORY, OpenSelected)
            .keywords(["document", "folder", "toggle"]),
        CommandDescriptor::new("Expand folder", CATEGORY, Expand).keywords(["open", "unfold"]),
        CommandDescriptor::new("Collapse folder", CATEGORY, Collapse).keywords(["close", "fold"]),
        CommandDescriptor::new("Select next item", CATEGORY, SelectNext).keywords(["down"]),
        CommandDescriptor::new("Select previous item", CATEGORY, SelectPrevious).keywords(["up"]),
        CommandDescriptor::new("Select first item", CATEGORY, SelectFirst).keywords(["top"]),
        CommandDescriptor::new("Select last item", CATEGORY, SelectLast).keywords(["bottom"]),
    ]
}

/// One thing in the library tree: a folder or a document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LibraryItem {
    /// A folder.
    Folder(FolderId),
    /// A document.
    Document(DocumentId),
}

/// What the panel asks of the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibraryPanelEvent {
    /// Open this document in the editor.
    OpenDocument {
        /// The document to open.
        id: DocumentId,
        /// True when focus should move to the editor (the user pressed enter); false when
        /// the panel keeps focus (a click, so the arrow keys keep working in the list).
        focus_editor: bool,
    },
    /// The user pressed escape in the panel: return focus to the editor.
    Dismissed,
}

#[derive(Clone, Debug)]
struct Row {
    item: LibraryItem,
    depth: usize,
    label: SharedString,
    expanded: bool,
    parent: Option<FolderId>,
}

/// The library sidebar: the tree of folders and documents.
///
/// **Status: skeleton** (see the module docs): the API is final, the rendering is a plain
/// list.
///
/// ```ignore
/// let panel = cx.new(|cx| LibraryPanel::new(library.clone(), cx));
/// cx.subscribe_in(&panel, window, |this: &mut Workspace, _panel, event, window, cx| match event {
///     LibraryPanelEvent::OpenDocument { id, focus_editor } => this.open(*id, *focus_editor, window, cx),
///     LibraryPanelEvent::Dismissed => this.focus_editor(window, cx),
/// });
/// // cmd-0:   window.focus(&panel.focus_handle(cx));
/// // cmd-n:   let folder = panel.read(cx).selected_folder();
/// //          let meta = library.update(cx, |l, cx| l.create_document(folder, &content, cx))?;
/// //          panel.update(cx, |p, cx| p.reveal_document(meta.id, cx));
/// // when the editor shows another document:
/// //          panel.update(cx, |p, cx| p.set_active_document(Some(id), cx));
/// ```
pub struct LibraryPanel {
    library: Entity<Library>,
    focus_handle: FocusHandle,
    expanded: HashSet<FolderId>,
    selected: Option<LibraryItem>,
    active_document: Option<DocumentId>,
    renaming: Option<LibraryItem>,
    rows: Vec<Row>,
    scroll_handle: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<LibraryPanelEvent> for LibraryPanel {}

impl Focusable for LibraryPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl LibraryPanel {
    /// Creates the panel over `library`. It follows the library's
    /// [`LibraryEvent::TreeChanged`] by itself. The expanded folders are restored from the
    /// [`EXPANDED_FOLDERS_SETTING`] setting and written back when they change.
    pub fn new(library: Entity<Library>, cx: &mut Context<Self>) -> Self {
        let expanded: Vec<FolderId> = library
            .read(cx)
            .setting_or(EXPANDED_FOLDERS_SETTING, Vec::new());
        let subscription = cx.subscribe(
            &library,
            |this: &mut Self, _library, event: &LibraryEvent, cx| {
                if matches!(event, LibraryEvent::TreeChanged) {
                    this.tree_changed(cx);
                }
            },
        );
        let mut this = Self {
            library,
            focus_handle: cx.focus_handle(),
            expanded: expanded.into_iter().collect(),
            selected: None,
            active_document: None,
            renaming: None,
            rows: Vec::new(),
            scroll_handle: ScrollHandle::new(),
            _subscriptions: vec![subscription],
        };
        this.rebuild_rows(cx);
        this
    }

    /// The library this panel shows.
    pub fn library(&self) -> &Entity<Library> {
        &self.library
    }

    // ----- selection ------------------------------------------------------------------

    /// The selected row, if any.
    pub fn selected(&self) -> Option<LibraryItem> {
        self.selected
    }

    /// The folder a new document or folder should be created in: the selected folder
    /// itself, or the folder that holds the selected document. `None` is the root (also
    /// when nothing is selected).
    pub fn selected_folder(&self) -> Option<FolderId> {
        match self.selected? {
            LibraryItem::Folder(id) => Some(id),
            LibraryItem::Document(_) => self.selected_row().and_then(|row| row.parent),
        }
    }

    /// Selects `item` (or clears the selection with `None`) without expanding anything.
    /// An item that is not in the live tree clears the selection. Use
    /// [`reveal`](Self::reveal) to also make it visible.
    pub fn select(&mut self, item: Option<LibraryItem>, cx: &mut Context<Self>) {
        let tree = self.library.read(cx).tree();
        self.selected = item.filter(|item| ancestors(&tree, *item).is_some());
        self.scroll_to_selected();
        cx.notify();
    }

    /// Expands the folders that lead to `item`, selects it and scrolls it into view.
    /// Returns false (and changes nothing) when the item is not in the live tree.
    pub fn reveal(&mut self, item: LibraryItem, cx: &mut Context<Self>) -> bool {
        let tree = self.library.read(cx).tree();
        let Some(path) = ancestors(&tree, item) else {
            return false;
        };
        let before = self.expanded.len();
        self.expanded.extend(path);
        if self.expanded.len() != before {
            self.persist_expanded(cx);
        }
        self.selected = Some(item);
        self.rebuild_rows(cx);
        self.scroll_to_selected();
        cx.notify();
        true
    }

    /// [`reveal`](Self::reveal) for a document.
    pub fn reveal_document(&mut self, id: DocumentId, cx: &mut Context<Self>) -> bool {
        self.reveal(LibraryItem::Document(id), cx)
    }

    /// [`reveal`](Self::reveal) for a folder.
    pub fn reveal_folder(&mut self, id: FolderId, cx: &mut Context<Self>) -> bool {
        self.reveal(LibraryItem::Folder(id), cx)
    }

    /// The document currently shown in the editor, as told by the app.
    pub fn active_document(&self) -> Option<DocumentId> {
        self.active_document
    }

    /// Tells the panel which document the editor shows, so its row is marked. Does not
    /// change the selection; call [`reveal_document`](Self::reveal_document) for that.
    pub fn set_active_document(&mut self, id: Option<DocumentId>, cx: &mut Context<Self>) {
        if self.active_document != id {
            self.active_document = id;
            cx.notify();
        }
    }

    // ----- expansion ------------------------------------------------------------------

    /// Whether a folder is expanded.
    pub fn is_expanded(&self, folder: FolderId) -> bool {
        self.expanded.contains(&folder)
    }

    /// Expands or collapses a folder. Collapsing a folder that hides the selection moves
    /// the selection to that folder.
    pub fn set_expanded(&mut self, folder: FolderId, expanded: bool, cx: &mut Context<Self>) {
        let changed = if expanded {
            self.expanded.insert(folder)
        } else {
            self.expanded.remove(&folder)
        };
        if !changed {
            return;
        }
        self.persist_expanded(cx);
        self.rebuild_rows(cx);
        if self.selected.is_some() && self.selected_index().is_none() {
            self.selected = Some(LibraryItem::Folder(folder));
        }
        cx.notify();
    }

    /// The rows currently visible, top to bottom (folders that are collapsed hide their
    /// contents).
    pub fn visible_items(&self) -> Vec<LibraryItem> {
        self.rows.iter().map(|row| row.item).collect()
    }

    // ----- rename ---------------------------------------------------------------------

    /// Starts renaming `item` in place: reveals and selects it and focuses the panel. The
    /// app calls this right after creating a folder so the user can type its name.
    /// Returns false when the item is not in the live tree.
    ///
    /// Skeleton: the state is tracked ([`renaming`](Self::renaming)) but the inline text
    /// field is drawn by the panel implementation.
    pub fn begin_rename(
        &mut self,
        item: LibraryItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.reveal(item, cx) {
            return false;
        }
        self.renaming = Some(item);
        window.focus(&self.focus_handle);
        cx.notify();
        true
    }

    /// The item being renamed in place, if any.
    pub fn renaming(&self) -> Option<LibraryItem> {
        self.renaming
    }

    /// Leaves rename mode without changing the name.
    pub fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        if self.renaming.take().is_some() {
            cx.notify();
        }
    }

    // ----- internals ------------------------------------------------------------------

    fn selected_index(&self) -> Option<usize> {
        let selected = self.selected?;
        self.rows.iter().position(|row| row.item == selected)
    }

    fn selected_row(&self) -> Option<&Row> {
        self.selected_index().map(|ix| &self.rows[ix])
    }

    fn scroll_to_selected(&self) {
        if let Some(ix) = self.selected_index() {
            self.scroll_handle.scroll_to_item(ix);
        }
    }

    fn persist_expanded(&mut self, cx: &mut Context<Self>) {
        let mut ids: Vec<FolderId> = self.expanded.iter().copied().collect();
        ids.sort();
        // A failed write only loses the expansion state across restarts.
        let _ = self.library.update(cx, |library, cx| {
            library.set_setting(EXPANDED_FOLDERS_SETTING, &ids, cx)
        });
    }

    fn rebuild_rows(&mut self, cx: &mut Context<Self>) {
        fn push(
            nodes: &[FolderNode],
            depth: usize,
            parent: Option<FolderId>,
            expanded: &HashSet<FolderId>,
            rows: &mut Vec<Row>,
        ) {
            for node in nodes {
                let open = expanded.contains(&node.folder.id);
                rows.push(Row {
                    item: LibraryItem::Folder(node.folder.id),
                    depth,
                    label: node.folder.name.clone().into(),
                    expanded: open,
                    parent,
                });
                if open {
                    push(
                        &node.folders,
                        depth + 1,
                        Some(node.folder.id),
                        expanded,
                        rows,
                    );
                    push_documents(&node.documents, depth + 1, Some(node.folder.id), rows);
                }
            }
        }
        fn push_documents(
            documents: &[writer_store::DocumentMeta],
            depth: usize,
            parent: Option<FolderId>,
            rows: &mut Vec<Row>,
        ) {
            for document in documents {
                let title = document.title.trim();
                rows.push(Row {
                    item: LibraryItem::Document(document.id),
                    depth,
                    label: if title.is_empty() {
                        "Untitled".into()
                    } else {
                        title.to_string().into()
                    },
                    expanded: false,
                    parent,
                });
            }
        }
        let tree = self.library.read(cx).tree();
        let mut rows = Vec::new();
        push(&tree.folders, 0, None, &self.expanded, &mut rows);
        push_documents(&tree.documents, 0, None, &mut rows);
        self.rows = rows;
    }

    /// The library's tree changed: rebuild the rows and repair the selection.
    fn tree_changed(&mut self, cx: &mut Context<Self>) {
        let old_index = self.selected_index();
        self.rebuild_rows(cx);
        let tree = self.library.read(cx).tree();
        if self
            .selected
            .is_some_and(|item| ancestors(&tree, item).is_none())
        {
            // The selected item is gone: stay at the same place in the list.
            self.selected = old_index
                .and_then(|ix| self.rows.get(ix.min(self.rows.len().saturating_sub(1))))
                .map(|row| row.item);
        }
        if self
            .renaming
            .is_some_and(|item| ancestors(&tree, item).is_none())
        {
            self.renaming = None;
        }
        if self
            .active_document
            .is_some_and(|id| ancestors(&tree, LibraryItem::Document(id)).is_none())
        {
            self.active_document = None;
        }
        cx.notify();
    }

    fn select_index(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.get(ix) {
            self.selected = Some(row.item);
            self.scroll_handle.scroll_to_item(ix);
            cx.notify();
        }
    }

    fn select_next(&mut self, _: &SelectNext, _window: &mut Window, cx: &mut Context<Self>) {
        let next = match self.selected_index() {
            Some(ix) => (ix + 1).min(self.rows.len().saturating_sub(1)),
            None => 0,
        };
        self.select_index(next, cx);
    }

    fn select_previous(
        &mut self,
        _: &SelectPrevious,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous = match self.selected_index() {
            Some(ix) => ix.saturating_sub(1),
            None => self.rows.len().saturating_sub(1),
        };
        self.select_index(previous, cx);
    }

    fn select_first(&mut self, _: &SelectFirst, _window: &mut Window, cx: &mut Context<Self>) {
        self.select_index(0, cx);
    }

    fn select_last(&mut self, _: &SelectLast, _window: &mut Window, cx: &mut Context<Self>) {
        self.select_index(self.rows.len().saturating_sub(1), cx);
    }

    fn expand(&mut self, _: &Expand, _window: &mut Window, cx: &mut Context<Self>) {
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
                self.select_index(ix + 1, cx);
            }
        }
    }

    fn collapse(&mut self, _: &Collapse, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.selected_row().cloned() else {
            return;
        };
        match row.item {
            LibraryItem::Folder(id) if row.expanded => self.set_expanded(id, false, cx),
            _ => {
                if let Some(parent) = row.parent {
                    self.selected = Some(LibraryItem::Folder(parent));
                    self.scroll_to_selected();
                    cx.notify();
                }
            }
        }
    }

    fn open_selected(&mut self, _: &OpenSelected, _window: &mut Window, cx: &mut Context<Self>) {
        match self.selected_row().map(|row| (row.item, row.expanded)) {
            Some((LibraryItem::Document(id), _)) => cx.emit(LibraryPanelEvent::OpenDocument {
                id,
                focus_editor: true,
            }),
            Some((LibraryItem::Folder(id), expanded)) => self.set_expanded(id, !expanded, cx),
            None => {}
        }
    }

    fn dismiss(&mut self, _: &Dismiss, _window: &mut Window, cx: &mut Context<Self>) {
        if self.renaming.is_some() {
            self.cancel_rename(cx);
        } else {
            cx.emit(LibraryPanelEvent::Dismissed);
        }
    }

    fn click_row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(ix).cloned() else {
            return;
        };
        window.focus(&self.focus_handle);
        self.selected = Some(row.item);
        match row.item {
            LibraryItem::Document(id) => cx.emit(LibraryPanelEvent::OpenDocument {
                id,
                focus_editor: false,
            }),
            LibraryItem::Folder(id) => self.set_expanded(id, !row.expanded, cx),
        }
        cx.notify();
    }
}

/// The folders that lead to `item`, outermost first, or `None` when it is not in the tree.
fn ancestors(tree: &LibraryTree, item: LibraryItem) -> Option<Vec<FolderId>> {
    fn walk(nodes: &[FolderNode], item: LibraryItem, path: &mut Vec<FolderId>) -> bool {
        for node in nodes {
            if item == LibraryItem::Folder(node.folder.id) {
                return true;
            }
            path.push(node.folder.id);
            let here = node
                .documents
                .iter()
                .any(|document| item == LibraryItem::Document(document.id));
            if here || walk(&node.folders, item, path) {
                return true;
            }
            path.pop();
        }
        false
    }
    if tree
        .documents
        .iter()
        .any(|document| item == LibraryItem::Document(document.id))
    {
        return Some(Vec::new());
    }
    let mut path = Vec::new();
    walk(&tree.folders, item, &mut path).then_some(path)
}

impl Render for LibraryPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors;
        let focused = self.focus_handle.is_focused(window);
        let selected = self.selected;
        let active = self.active_document;

        let rows = self.rows.iter().enumerate().map(|(ix, row)| {
            let is_selected = selected == Some(row.item);
            let is_active = matches!(row.item, LibraryItem::Document(id) if active == Some(id));
            let icon = match row.item {
                LibraryItem::Folder(_) if row.expanded => IconName::FolderOpen,
                LibraryItem::Folder(_) => IconName::Folder,
                LibraryItem::Document(_) => IconName::Document,
            };
            div()
                .id(("library-row", ix))
                .flex()
                .flex_none()
                .items_center()
                .gap(SPACE_2)
                .h(px(28.))
                .mx(SPACE_1)
                .pr(SPACE_2)
                .pl(SPACE_2 + SPACE_4 * row.depth as f32)
                .rounded(RADIUS_SM)
                .when(is_selected, |el| {
                    el.bg(if focused {
                        colors.accent_subtle
                    } else {
                        colors.hover
                    })
                })
                .when(!is_selected, |el| el.hover(|style| style.bg(colors.hover)))
                .child(Icon::new(icon).color(if is_active {
                    colors.accent
                } else {
                    colors.muted
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(if is_active { colors.accent } else { colors.ink })
                        .child(row.label.clone()),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.click_row(ix, window, cx)
                }))
        });
        let rows: Vec<_> = rows.collect();
        let empty = rows.is_empty();

        div()
            .id("library-panel")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::select_first))
            .on_action(cx.listener(Self::select_last))
            .on_action(cx.listener(Self::expand))
            .on_action(cx.listener(Self::collapse))
            .on_action(cx.listener(Self::open_selected))
            .on_action(cx.listener(Self::dismiss))
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.sidebar)
            .text_color(colors.ink)
            .text_size(UI_TEXT_BASE)
            .when(empty, |el| {
                el.child(
                    div()
                        .p(SPACE_4)
                        .text_color(colors.muted)
                        .child("No documents yet."),
                )
            })
            .child(
                div()
                    .id("library-rows")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .py(SPACE_1)
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll_handle)
                    .children(rows),
            )
    }
}
