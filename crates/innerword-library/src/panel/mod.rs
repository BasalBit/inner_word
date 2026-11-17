//! [`LibraryPanel`]: the library sidebar, where the user organises their writing into
//! folders. Everything in it works from the keyboard, and everything in it works with the
//! mouse alone.
//!
//! The keys, the mouse and the mounting recipe are in the docs of [`LibraryPanel`].

mod controls;
mod drag;
mod handlers;
mod menu;
mod move_picker;
mod render;
pub(crate) mod rows;
mod trash;
mod undo;

use std::cell::Cell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, Bounds, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    KeyBinding, Pixels, ScrollStrategy, SharedString, Subscription, Task, UniformListScrollHandle,
    Window, actions, prelude::*, px,
};
use serde::{Deserialize, Serialize};
use innerword_store::{DocumentContent, DocumentId, FolderId, Result};
use innerword_ui::context_menu::OpenContextMenu;
use innerword_ui::toast::show_toast;
use innerword_ui::{
    CommandDescriptor, ConfirmDialog, FieldAppearance, IconName, ModalLayer, Picker, TextField,
    TextFieldEvent, Toast,
};

use crate::library::{Library, LibraryEvent};
use drag::DropTarget;
pub use menu::LibraryMenuTarget;
use menu::MenuExtension;
use move_picker::MoveDelegate;
use rows::{
    Filter, Row, ancestors, build_rows, count_documents, count_folders, item_name, place_of,
    plural, short, short_path, stored_name,
};
use trash::{ROOT_LABEL, TrashRow, build_trash_rows};
use undo::UndoEntry;

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
        /// Move the selection up by about a screenful of rows.
        SelectPageUp,
        /// Move the selection down by about a screenful of rows.
        SelectPageDown,
        /// Expand the selected folder, or step into it when it is already expanded.
        Expand,
        /// Collapse the selected folder, or step out to the parent folder.
        Collapse,
        /// Open the selected document in the editor, or toggle the selected folder.
        OpenSelected,
        /// Leave the panel (the app returns focus to the editor).
        Dismiss,
        /// Create a document in the selected folder and name it in place.
        NewDocument,
        /// Create a folder in the selected folder and name it in place.
        NewFolder,
        /// Rename the selected item in place.
        Rename,
        /// Duplicate the selected document, or the selected folder with everything in it.
        Duplicate,
        /// Move the selected item to another folder, chosen from a list.
        MoveToFolder,
        /// Move the selected item one place up among its siblings.
        MoveUp,
        /// Move the selected item one place down among its siblings.
        MoveDown,
        /// Move the selected item to the trash.
        MoveToTrash,
        /// Take back the last change made in the library: a move to the trash, a rename,
        /// a move to another folder or a reorder.
        UndoTrash,
        /// Focus the filter field of the library.
        FocusFilter,
        /// Return from a text field of the panel to the tree.
        FocusTree,
        /// Show the trash.
        ShowTrash,
        /// Restore the selected item from the trash.
        RestoreSelected,
        /// Delete the selected trashed item for good.
        DeleteForever,
        /// Delete everything in the trash for good.
        EmptyTrash,
        /// Switch the detail on document rows between edit time and word count.
        ToggleRowDetail,
        /// Expand the selected folder and every folder inside it. With anything else
        /// selected (a document, the "Library" row): every folder of the library.
        ExpandAll,
        /// Collapse the selected folder and every folder inside it. With anything else
        /// selected (a document, the "Library" row): every folder of the library.
        CollapseAll,
    ]
);

/// The key context of the panel's root element; use it in binding predicates. The context
/// also carries `mode`, which is `tree` or `trash`.
pub const KEY_CONTEXT: &str = "LibraryPanel";

/// The settings key under which the panel stores the ids of the expanded folders
/// (a JSON array of folder ids).
pub const EXPANDED_FOLDERS_SETTING: &str = "library.expanded_folders";

/// The settings key under which the panel stores the selected item
/// (`{"Folder": 3}`, `{"Document": 12}` or `null`). Written a moment after the selection
/// settles, and on quit.
pub const SELECTED_ITEM_SETTING: &str = "library.selected_item";

/// The settings key of the [`RowDetail`] (`"edited"` or `"words"`).
pub const ROW_DETAIL_SETTING: &str = "library.row_detail";

/// The height of one row of the tree.
pub const ROW_HEIGHT: Pixels = px(26.);
/// The height of the strip above the list: the filter row, or the title of the trash view.
pub const HEADER_HEIGHT: Pixels = px(34.);
/// The height of one row of the trash view (name over original location).
pub const TRASH_ROW_HEIGHT: Pixels = px(44.);
/// The height of the strip between the header and the tree that holds the "Library" row
/// (the root of the tree, selectable like any row).
pub const ROOT_ROW_HEIGHT: Pixels = px(30.);

const SELECTION_PERSIST_DELAY: Duration = Duration::from_millis(400);
/// How long the panel waits to be drawn after it was given the keyboard while off screen,
/// before it gives the keyboard back. Checked twice.
const OFF_SCREEN_GRACE: Duration = Duration::from_millis(250);
const NEW_FOLDER_NAME: &str = "New folder";
/// How much of a name a toast shows. A toast is a single line of about 78 characters; the
/// longest sentences here have about 50 of their own.
const NAME_IN_MESSAGE: usize = 26;
/// How much of a folder path a toast shows (the end of it).
const PLACE_IN_MESSAGE: usize = 30;

/// The panel's default key bindings, all scoped to [`KEY_CONTEXT`]. Installed by
/// [`crate::init`].
///
/// Where one action has several keys, the canonical key comes last: key hints show the
/// binding added last.
pub fn key_bindings() -> Vec<KeyBinding> {
    // Everywhere in the panel. In a text field the field's own bindings for the same keys
    // win (they are deeper), unless the field passes the key on.
    let panel = Some(KEY_CONTEXT);
    // Not while typing in the filter or renaming.
    let list = Some("LibraryPanel && !TextField");
    let tree = Some("LibraryPanel && mode == tree && !TextField");
    let trash = Some("LibraryPanel && mode == trash && !TextField");
    vec![
        KeyBinding::new("ctrl-n", SelectNext, panel),
        KeyBinding::new("j", SelectNext, list),
        KeyBinding::new("down", SelectNext, panel),
        KeyBinding::new("ctrl-p", SelectPrevious, panel),
        KeyBinding::new("k", SelectPrevious, list),
        KeyBinding::new("up", SelectPrevious, panel),
        KeyBinding::new("g g", SelectFirst, list),
        KeyBinding::new("cmd-up", SelectFirst, panel),
        KeyBinding::new("home", SelectFirst, panel),
        KeyBinding::new("shift-g", SelectLast, list),
        KeyBinding::new("cmd-down", SelectLast, panel),
        KeyBinding::new("end", SelectLast, panel),
        KeyBinding::new("pageup", SelectPageUp, panel),
        KeyBinding::new("pagedown", SelectPageDown, panel),
        KeyBinding::new("l", Expand, list),
        KeyBinding::new("right", Expand, panel),
        KeyBinding::new("h", Collapse, list),
        KeyBinding::new("left", Collapse, panel),
        KeyBinding::new("enter", OpenSelected, panel),
        KeyBinding::new("escape", Dismiss, panel),
        KeyBinding::new("tab", FocusTree, panel),
        KeyBinding::new("n", NewDocument, tree),
        KeyBinding::new("shift-n", NewFolder, tree),
        KeyBinding::new("f2", Rename, tree),
        KeyBinding::new("r", Rename, tree),
        KeyBinding::new("cmd-d", Duplicate, tree),
        KeyBinding::new("m", MoveToFolder, tree),
        KeyBinding::new("alt-up", MoveUp, tree),
        KeyBinding::new("alt-down", MoveDown, tree),
        KeyBinding::new("cmd-backspace", MoveToTrash, tree),
        KeyBinding::new("cmd-z", UndoTrash, list),
        KeyBinding::new("cmd-f", FocusFilter, tree),
        KeyBinding::new("/", FocusFilter, tree),
        KeyBinding::new("t", ShowTrash, tree),
        KeyBinding::new("i", ToggleRowDetail, tree),
        KeyBinding::new("alt-right", ExpandAll, tree),
        KeyBinding::new("alt-left", CollapseAll, tree),
        // The key macOS itself uses for context menus, and before it (so
        // that key hints show `ctrl-enter`) the one other systems use.
        KeyBinding::new("shift-f10", OpenContextMenu, list),
        KeyBinding::new("ctrl-enter", OpenContextMenu, list),
        KeyBinding::new("cmd-shift-backspace", EmptyTrash, list),
        // After the generic bindings, so these win in the trash view.
        KeyBinding::new("enter", RestoreSelected, trash),
        KeyBinding::new("r", RestoreSelected, trash),
        KeyBinding::new("cmd-backspace", DeleteForever, trash),
    ]
}

/// The panel's commands for the command palette and the shortcuts overlay.
///
/// Their actions are handled by the panel, so they act when the panel has focus. Wrap the
/// window root with [`forward_actions`] to make the commands that do not need the panel's
/// focus (new, rename, duplicate, move, trash, filter, ...) work from anywhere.
pub fn commands() -> Vec<CommandDescriptor> {
    const CATEGORY: &str = "Library";
    vec![
        CommandDescriptor::new("New document", CATEGORY, NewDocument)
            .keywords(["create", "add", "file"]),
        CommandDescriptor::new("New folder", CATEGORY, NewFolder)
            .keywords(["create", "add", "group"]),
        CommandDescriptor::new("Rename", CATEGORY, Rename).keywords(["name", "title"]),
        CommandDescriptor::new("Duplicate", CATEGORY, Duplicate)
            .keywords(["copy", "clone", "document", "folder"]),
        CommandDescriptor::new("Move to folder", CATEGORY, MoveToFolder)
            .keywords(["relocate", "organise", "organize"]),
        CommandDescriptor::new("Move up", CATEGORY, MoveUp).keywords(["reorder", "sort"]),
        CommandDescriptor::new("Move down", CATEGORY, MoveDown).keywords(["reorder", "sort"]),
        CommandDescriptor::new("Move to trash", CATEGORY, MoveToTrash)
            .keywords(["delete", "remove", "bin"]),
        CommandDescriptor::new("Undo library change", CATEGORY, UndoTrash).keywords([
            "restore",
            "undelete",
            "bring back",
            "trash",
            "rename",
            "move",
        ]),
        CommandDescriptor::new("Show trash", CATEGORY, ShowTrash)
            .keywords(["deleted", "bin", "restore"]),
        CommandDescriptor::new("Restore from trash", CATEGORY, RestoreSelected)
            .keywords(["undelete", "put back"]),
        CommandDescriptor::new("Delete forever", CATEGORY, DeleteForever)
            .keywords(["purge", "erase", "trash"]),
        CommandDescriptor::new("Empty trash", CATEGORY, EmptyTrash).keywords([
            "purge",
            "delete forever",
            "bin",
        ]),
        CommandDescriptor::new("Filter by name", CATEGORY, FocusFilter)
            .keywords(["search", "find", "narrow"]),
        CommandDescriptor::new("Switch row detail", CATEGORY, ToggleRowDetail).keywords([
            "word count",
            "edit time",
            "modified",
        ]),
        CommandDescriptor::new("Open selected item", CATEGORY, OpenSelected)
            .keywords(["document", "folder", "toggle"]),
        CommandDescriptor::new("Expand folder", CATEGORY, Expand).keywords(["open", "unfold"]),
        CommandDescriptor::new("Collapse folder", CATEGORY, Collapse).keywords(["close", "fold"]),
        CommandDescriptor::new("Expand all folders", CATEGORY, ExpandAll).keywords([
            "open",
            "unfold",
            "subfolders",
            "everything",
            "inside",
        ]),
        CommandDescriptor::new("Collapse all folders", CATEGORY, CollapseAll).keywords([
            "close",
            "fold",
            "subfolders",
            "everything",
            "inside",
        ]),
        CommandDescriptor::new("Select next item", CATEGORY, SelectNext).keywords(["down"]),
        CommandDescriptor::new("Select previous item", CATEGORY, SelectPrevious).keywords(["up"]),
        CommandDescriptor::new("Select first item", CATEGORY, SelectFirst).keywords(["top"]),
        CommandDescriptor::new("Select last item", CATEGORY, SelectLast).keywords(["bottom"]),
    ]
}

/// Makes the panel's commands work when focus is somewhere else in the window (the editor,
/// after the command palette closed): adds listeners to `element` that hand the panel's
/// command actions to `panel`. Use it on the window's root element.
///
/// When the panel itself has focus its own listeners run first, so nothing runs twice.
/// Covered: new document, new folder, rename, duplicate, move to folder, move up / down,
/// move to trash, undo, show trash, empty trash, filter, switch row detail, expand all,
/// collapse all.
///
/// A command for one item (rename, duplicate, move to folder, move up / down, move to
/// trash) that arrives while the keyboard is outside the panel acts on the document open
/// in the editor ([`LibraryPanel::set_active_document`]), whose row is selected first:
/// while writing, "Move to trash" means the text in front of the user, not a row the
/// sidebar happens to be on. With the keyboard in the panel it acts on the selected row.
///
/// Some of these put the keyboard in the panel (a rename field, the filter, the trash
/// view). When the panel is not on screen at that moment, it asks its host to show it:
/// see [`LibraryPanel::set_reveal_handler`].
///
/// ```ignore
/// innerword_library::forward_actions(div().size_full(), &self.panel).child(..)
/// ```
pub fn forward_actions<E: InteractiveElement>(element: E, panel: &Entity<LibraryPanel>) -> E {
    macro_rules! forward {
        ($element:expr, $($action:ty => $method:ident),* $(,)?) => {{
            let element = $element;
            $(
                let element = element.on_action({
                    let panel = panel.clone();
                    move |action: &$action, window: &mut Window, cx: &mut App| {
                        panel.update(cx, |panel, cx| panel.$method(action, window, cx));
                    }
                });
            )*
            element
        }};
    }
    forward!(
        element,
        NewDocument => on_new_document,
        NewFolder => on_new_folder,
        Rename => on_rename,
        Duplicate => on_duplicate,
        MoveToFolder => on_move_to_folder,
        MoveUp => on_move_up,
        MoveDown => on_move_down,
        MoveToTrash => on_move_to_trash,
        UndoTrash => on_undo_trash,
        ShowTrash => on_show_trash,
        EmptyTrash => on_empty_trash,
        FocusFilter => on_focus_filter,
        ToggleRowDetail => on_toggle_row_detail,
        ExpandAll => on_expand_all,
        CollapseAll => on_collapse_all,
    )
}

/// One thing in the library tree: a folder or a document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
        /// the panel keeps focus (a click, or a document that was just created and named
        /// in the panel, so the arrow keys keep working in the list).
        focus_editor: bool,
    },
    /// The user pressed escape in the panel: return focus to the editor.
    Dismissed,
    /// Files dropped on the panel: on a folder row (`folder`), else the folder selected, else the root.
    /// The variant and the app's empty arm are WP4's (wave 1); WP6 emits it.
    DropPaths {
        /// The paths that were dropped, as the system handed them over.
        paths: Vec<PathBuf>,
        /// The folder they were dropped into; `None` is the root.
        folder: Option<FolderId>,
    },
}

/// The muted detail at the right of a document row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RowDetail {
    /// How long ago the document was edited: "now", "5m", "3h", "2d", "3w", "5mo", "2y".
    #[default]
    Edited,
    /// The document's word count: "1,240".
    Words,
}

/// How the panel asks its host to put it on screen.
type RevealHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// Where parts of the panel were drawn in the last frame, in window coordinates: a menu
/// opened from the keyboard appears at the thing it belongs to.
#[derive(Default)]
struct Anchors {
    add_button: Cell<Bounds<Pixels>>,
    root_row: Cell<Bounds<Pixels>>,
    trash_row: Cell<Bounds<Pixels>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Tree,
    Trash,
}

/// The library sidebar: the tree of folders and documents, the name filter and the trash.
/// Everything in it works from the keyboard, and everything in it works with the mouse
/// alone.
///
/// # What it shows
///
/// - A filter field (`/`), which narrows the tree by name as you type.
/// - The tree: nested folders first, then documents, in the user's own order. A page (an
///   HTML page kept in the library) is a document row with a globe for its icon. Folders
///   expand and collapse. The document open in the editor is marked. Each document row has
///   a muted detail: how long ago it was edited, or its word count
///   ([`RowDetail`], switched with `i`).
/// - A "Library" row above the tree: the root. `up` from the first row or `left` from a
///   top-level row selects it; with it selected, new documents and folders are created at
///   the top level.
/// - A "Trash" row at the bottom. It opens the trash view: the trashed items with the place
///   they came from, where they can be restored or deleted for good.
///
/// # Keys
///
/// All bindings live in the [`KEY_CONTEXT`] key context ([`key_bindings`]); the plain-letter
/// ones are switched off while a text field in the panel has focus.
///
/// | Keys | Tree | Trash view |
/// |---|---|---|
/// | `up` / `down`, `k` / `j`, `ctrl-p` / `ctrl-n` | move the selection | same |
/// | `home` / `end`, `cmd-up` / `cmd-down`, `g g` / `shift-g` | first / last row | same |
/// | `pageup` / `pagedown` | move by a screenful | same |
/// | `left` / `h` | collapse the folder, or go to the parent (at the top level: the "Library" row) | back to the tree |
/// | `right` / `l` | expand the folder, or go to its first child | |
/// | `enter` | open the document (focus goes to the editor), or toggle the folder | restore |
/// | `n` / `shift-n` | new document / new folder in the selected folder (on the "Library" row: at the top level), then rename it | |
/// | `r` / `f2` | rename in place (`enter` commits, `escape` cancels) | `r` restores |
/// | `cmd-d` | duplicate the document, or the folder with everything in it | |
/// | `m` | move to another folder (a picker, with "Library root") | |
/// | `alt-up` / `alt-down` | reorder among siblings | |
/// | `cmd-backspace` | move to the trash (a toast offers undo) | delete forever (asks first) |
/// | `cmd-z` | undo the last move to the trash, rename, move or reorder | same |
/// | `/` / `cmd-f` | focus the filter (`escape` clears it, `tab` keeps it and returns to the tree, `enter` opens the selected match) | |
/// | `t` | show the trash | |
/// | `cmd-shift-backspace` | empty the trash (asks first) | same |
/// | `i` | switch the row detail between edit time and word count | |
/// | `alt-right` / `alt-left` | expand / collapse the selected folder and every folder in it (on a document or the "Library" row: every folder) | |
/// | `ctrl-enter` | the context menu of the selected row, at the row | of the selected item |
/// | `escape` | clear the filter, else leave the panel ([`LibraryPanelEvent::Dismissed`]) | back to the tree |
///
/// # Mouse
///
/// - A click opens a document (the list keeps the keyboard) and opens or closes a folder;
///   the triangle of a folder does the same without selecting it. A double click renames
///   in place.
/// - The "+" button of the header opens a menu with "New Document" and "New Folder". They
///   go where `n` and `shift-n` put them: into the selected folder, or to the top level.
/// - A right click selects what was clicked and opens its context menu; so does the "more"
///   button at the right end of the selected row and of the row under the pointer, and
///   `ctrl-enter`. The menus: on a document Open, Rename, Duplicate, Move To, Move to
///   Trash; on a folder also New Document Here, New Folder Here and Expand All or
///   Collapse All; on the "Library" row and on the empty area under the rows New
///   Document, New Folder, Expand All or Collapse All, Show Trash; on the "Trash" row
///   Show Trash and Empty Trash; in the trash view Restore and Delete Forever on an item,
///   Back to Library and Empty Trash beside the items. Every item is one of the panel's
///   actions, shows its key and does what the key does. The app adds its own items with
///   [`set_menu_extension`](Self::set_menu_extension).
/// - Drag and drop: a row dragged onto a folder moves into it (a closed folder opens
///   when the drag rests on it), dragged between two rows it takes that place among its
///   siblings, onto the "Library" row or the empty area it moves to the top level, and
///   onto the "Trash" row to the trash. A line or a frame shows where it would land.
///   A folder cannot go into itself or into a folder inside it. The moves are the ones
///   the keys make (`m`, `alt-up` / `alt-down`, `cmd-backspace`) and `cmd-z` takes them
///   back. Under a filter only moves into folders are offered. Close to the top or
///   bottom edge of the list a drag scrolls it.
/// - Files dragged in from outside the app (the Finder) can be dropped anywhere on the
///   panel: the panel takes none of them itself and emits
///   [`LibraryPanelEvent::DropPaths`] with the folder they were dropped into (a folder
///   row: that folder; a document row: its folder; the "Library" row: the top level;
///   anywhere else: the folder selected, else the top level), which is marked while they
///   are over it. The app decides what to take (`.html` files and folders become pages).
/// - The trash view has its actions as buttons under the list, and "Back" above it; the
///   empty library offers "New document" and "New folder" as buttons.
///
/// Context menus are drawn by `innerword_ui::ContextMenuLayer`, which the toast layer of the
/// window brings along.
///
/// # Mounting
///
/// ```ignore
/// innerword_library::init(cx);                               // key bindings, after the app's own
/// let panel = cx.new(|cx| LibraryPanel::new(library.clone(), cx));
/// panel.update(cx, |panel, _| {
///     panel.set_modal_layer(modal_layer.clone());         // share the window's modal layer
///     panel.set_new_document_content(empty_document_json); // what `n` creates
/// });
/// // If the sidebar can be hidden: how the panel asks to be shown (a palette command
/// // that renames, filters or shows the trash while the sidebar is closed).
/// panel.update(cx, |panel, _| panel.set_reveal_handler(move |window, cx| { /* show the sidebar */ }));
/// // Toasts go through innerword_ui::toast::show_toast: install a global ToastLayer.
/// // In the window root, so palette commands reach the panel from anywhere:
/// innerword_library::forward_actions(div().size_full(), &panel)
/// ```
///
/// # Wiring to the app
///
/// ```ignore
/// cx.subscribe_in(&panel, window, |this: &mut Workspace, _panel, event, window, cx| match event {
///     LibraryPanelEvent::OpenDocument { id, focus_editor } => this.open(*id, *focus_editor, window, cx),
///     LibraryPanelEvent::Dismissed => this.focus_editor(window, cx),
/// });
/// // cmd-0:   window.focus(&panel.focus_handle(cx));
/// // cmd-n:   let folder = panel.read(cx).selected_folder();
/// //          let meta = library.update(cx, |l, cx| l.create_document(folder, &content, cx))?;
/// //          panel.update(cx, |p, cx| p.reveal_document(meta.id, cx));
/// // when the editor shows another document (the selection follows it):
/// //          panel.update(cx, |p, cx| p.set_active_document(Some(id), cx));
/// ```
///
/// Do not call the panel's methods from inside a `library.update(..)` closure: they read
/// the library (and some write settings through it), which GPUI forbids while the library
/// is being updated. Calling them from a [`LibraryEvent`] subscription is fine.
pub struct LibraryPanel {
    library: Entity<Library>,
    focus_handle: FocusHandle,
    view: View,

    // Tree.
    expanded: HashSet<FolderId>,
    selected: Option<LibraryItem>,
    /// The "Trash" row under the tree is selected (then `selected` is `None`).
    on_trash_row: bool,
    /// The "Library" row above the tree is selected (then `selected` is `None`).
    on_root_row: bool,
    active_document: Option<DocumentId>,
    rows: Vec<Row>,
    scroll: UniformListScrollHandle,
    detail: RowDetail,

    // Rename in place.
    renaming: Option<LibraryItem>,
    /// The item being renamed was just created with `n`: open it once it has a name.
    rename_opens: bool,
    /// The rename gives a new item its first name: nothing to undo.
    rename_is_naming: bool,
    rename_field: Option<Entity<TextField>>,

    // Filter.
    filter_text: String,
    filter: Option<Filter>,
    filter_field: Option<Entity<TextField>>,
    /// What was selected when the filter was switched on: where to go back to when it is
    /// cleared with nothing selected (it matched nothing).
    selection_before_filter: Option<LibraryItem>,

    // Trash.
    trash_rows: Vec<TrashRow>,
    trash_selected: Option<LibraryItem>,
    trash_scroll: UniformListScrollHandle,
    /// The changes of this session that `cmd-z` can take back, oldest first.
    undo_stack: Vec<UndoEntry>,

    // Mouse.
    /// During a drag: where the dragged item would land.
    drop_target: Option<DropTarget>,
    /// Opens the collapsed folder a drag rests on, after a moment.
    drop_expand: Option<Task<()>>,
    /// The folder that the last single click opened or closed. A second click (a double
    /// click renames) puts it back as it was.
    click_toggled: Option<FolderId>,
    /// Where the parts of the panel that a menu can open from were drawn.
    anchors: Rc<Anchors>,
    /// The app's own items for the panel's context menus.
    menu_extension: Option<MenuExtension>,
    /// The menu being opened was asked for with the key, not with the mouse.
    menu_from_keyboard: bool,

    new_document_content: Option<String>,
    modal_layer: Entity<ModalLayer>,
    /// The modal layer belongs to the app, which renders it.
    modal_layer_is_shared: bool,
    selection_dirty: bool,
    persist_selection: Option<Task<()>>,
    /// Whether the keyboard was in the panel when it was last drawn, or when the command
    /// being handled arrived (the handlers of commands that say a key set it).
    keyboard_inside: bool,
    /// Tracked on an empty element inside the panel, only to learn whether the panel was
    /// drawn in the last frame.
    probe: FocusHandle,
    /// Asks the host to put the panel on screen.
    reveal_handler: Option<RevealHandler>,
    /// Waits for the panel to be drawn after it took the keyboard while off screen.
    off_screen_watch: Option<Task<()>>,
    focus_subscription: Option<Subscription>,
    modal_subscription: Subscription,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<LibraryPanelEvent> for LibraryPanel {}

impl Focusable for LibraryPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl LibraryPanel {
    /// The key that opens the menu of a row, as the "more" controls of the rows name it:
    /// the binding of the shared `innerword_ui::context_menu::OpenContextMenu` that fires in
    /// the panel, as the keymap has it now. (The comment list, the text fields and the
    /// chats bind the same action, each with a key of its own.)
    pub fn context_menu_key(&self, window: &Window, cx: &App) -> Option<innerword_ui::KeyHint> {
        innerword_ui::KeyHint::for_action_at(&OpenContextMenu, &self.focus_handle, window, cx)
    }
}

impl LibraryPanel {
    /// Creates the panel over `library`. It follows the library's events by itself. The
    /// expanded folders, the selection and the row detail are restored from the settings
    /// ([`EXPANDED_FOLDERS_SETTING`], [`SELECTED_ITEM_SETTING`], [`ROW_DETAIL_SETTING`]) and
    /// written back when they change.
    pub fn new(library: Entity<Library>, cx: &mut Context<Self>) -> Self {
        let (expanded, stored_selection, detail, trash_rows) = {
            let library = library.read(cx);
            (
                library.setting_or::<Vec<FolderId>>(EXPANDED_FOLDERS_SETTING, Vec::new()),
                library.setting_or::<Option<LibraryItem>>(SELECTED_ITEM_SETTING, None),
                library.setting_or(ROW_DETAIL_SETTING, RowDetail::default()),
                build_trash_rows(library),
            )
        };
        let modal_layer = cx.new(|_| ModalLayer::new());
        let modal_subscription = cx.observe(&modal_layer, |_, _, cx| cx.notify());
        let subscriptions = vec![
            cx.subscribe(
                &library,
                |this: &mut Self, _library, event: &LibraryEvent, cx| this.library_event(event, cx),
            ),
            cx.on_app_quit(|this: &mut Self, cx| {
                this.persist_selection_now(cx);
                async {}
            }),
        ];
        let mut this = Self {
            library,
            focus_handle: cx.focus_handle(),
            view: View::Tree,
            expanded: expanded.into_iter().collect(),
            selected: None,
            on_trash_row: false,
            on_root_row: false,
            active_document: None,
            rows: Vec::new(),
            scroll: UniformListScrollHandle::new(),
            detail,
            renaming: None,
            rename_opens: false,
            rename_is_naming: false,
            rename_field: None,
            filter_text: String::new(),
            filter: None,
            filter_field: None,
            selection_before_filter: None,
            trash_rows,
            trash_selected: None,
            trash_scroll: UniformListScrollHandle::new(),
            undo_stack: Vec::new(),
            drop_target: None,
            drop_expand: None,
            click_toggled: None,
            anchors: Rc::default(),
            menu_extension: None,
            menu_from_keyboard: false,
            new_document_content: None,
            modal_layer,
            modal_layer_is_shared: false,
            selection_dirty: false,
            persist_selection: None,
            keyboard_inside: true,
            probe: cx.focus_handle(),
            reveal_handler: None,
            off_screen_watch: None,
            focus_subscription: None,
            modal_subscription,
            _subscriptions: subscriptions,
        };
        // Restore the selection if the item still exists, opening the way to it (in memory
        // only: the stored expansion is left as it is).
        if let Some(item) = stored_selection {
            let tree = this.library.read(cx).tree();
            if let Some(path) = ancestors(&tree, item) {
                this.expanded.extend(path);
                this.selected = Some(item);
            }
        }
        this.rebuild_rows(cx);
        this.scroll_to_selected(ScrollStrategy::Center);
        this
    }

    /// The library this panel shows.
    pub fn library(&self) -> &Entity<Library> {
        &self.library
    }

    // ----- wiring ---------------------------------------------------------------------

    /// The modal layer in which the panel shows its move picker and its confirmations. By
    /// default the panel owns one and draws it over the whole window.
    pub fn modal_layer(&self) -> &Entity<ModalLayer> {
        &self.modal_layer
    }

    /// Makes the panel use the window's own modal layer (which the app renders), so only
    /// one modal is ever open in the window.
    pub fn set_modal_layer(&mut self, layer: Entity<ModalLayer>, cx: &mut Context<Self>) {
        self.modal_subscription = cx.observe(&layer, |_, _, cx| cx.notify());
        self.modal_layer = layer;
        self.modal_layer_is_shared = true;
        cx.notify();
    }

    /// Sets the content JSON of an empty document, which `n` ([`NewDocument`]) stores in
    /// the documents it creates. The library never parses content, so the app supplies it.
    /// Until this is set the panel does not handle [`NewDocument`] and lets the action
    /// continue to the app's own handlers.
    pub fn set_new_document_content(&mut self, content: impl Into<String>) {
        self.new_document_content = Some(content.into());
    }

    /// Sets how the panel asks its host to put it on screen. A host that can hide the
    /// panel (a sidebar that closes) should set this.
    ///
    /// The panel calls it when something puts the keyboard inside the panel while the
    /// panel is not drawn: [`begin_rename`](Self::begin_rename) (so also
    /// [`new_document`](Self::new_document) and [`new_folder`](Self::new_folder)),
    /// [`focus_filter`](Self::focus_filter) and [`show_trash`](Self::show_trash), which is
    /// what the palette's library commands do when the sidebar is closed. The handler runs
    /// after the current update, so it may update the entity that called the panel.
    ///
    /// Without a handler, or when the panel still is not drawn a moment later, the panel
    /// ends the rename (keeping the name) and gives the keyboard back with
    /// [`LibraryPanelEvent::Dismissed`], so the keyboard is never left on something that is
    /// not on screen.
    pub fn set_reveal_handler(&mut self, handler: impl Fn(&mut Window, &mut App) + 'static) {
        self.reveal_handler = Some(Rc::new(handler));
    }

    /// Whether the panel was drawn in the window's last frame.
    pub fn is_on_screen(&self, window: &Window) -> bool {
        self.focus_handle.contains(&self.probe, window)
    }

    /// Gives the keyboard to `handle`, which is something inside the panel. When the panel
    /// is not on screen it asks the host to show it, and makes sure the keyboard does not
    /// stay on an element that is never drawn.
    fn focus_inside(&mut self, handle: &FocusHandle, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_on_screen(window) {
            if let Some(handler) = self.reveal_handler.clone() {
                // Deferred: the caller may be in the middle of updating the host.
                window.defer(cx, move |window, cx| handler(window, cx));
            }
            self.watch_off_screen(2, window, cx);
        }
        window.focus(handle);
    }

    /// Looks again in a moment whether the panel is drawn; after `checks` looks without
    /// it, gives the keyboard back.
    fn watch_off_screen(&mut self, checks: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.off_screen_watch = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(OFF_SCREEN_GRACE).await;
            this.update_in(cx, |this, window, cx| {
                this.off_screen_watch = None;
                if this.is_on_screen(window) {
                    return;
                }
                if checks > 1 {
                    window.refresh();
                    this.watch_off_screen(checks - 1, window, cx);
                } else {
                    this.leave_off_screen(window, cx);
                }
            })
            .ok();
        }));
    }

    /// The panel has the keyboard but is not drawn: end what needs the keyboard and hand
    /// it back to the host.
    fn leave_off_screen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_rename(true, cx);
        let focused = window.focused(cx);
        let ours = [
            Some(self.focus_handle.clone()),
            self.rename_field
                .as_ref()
                .map(|field| field.focus_handle(cx)),
            self.filter_field
                .as_ref()
                .map(|field| field.focus_handle(cx)),
        ];
        if focused.is_some() && ours.contains(&focused) {
            window.blur();
            cx.emit(LibraryPanelEvent::Dismissed);
        }
    }

    // ----- selection ------------------------------------------------------------------

    /// The selected row of the tree, if any (`None` also while the "Library" row or the
    /// "Trash" row is selected).
    pub fn selected(&self) -> Option<LibraryItem> {
        self.selected
    }

    /// The folder a new document or folder should be created in: the selected folder
    /// itself, or the folder that holds the selected document. `None` is the root (when
    /// the "Library" row is selected, and when nothing is).
    pub fn selected_folder(&self) -> Option<FolderId> {
        match self.selected? {
            LibraryItem::Folder(id) => Some(id),
            LibraryItem::Document(_) => self.selected_row().and_then(|row| row.parent),
        }
    }

    /// Whether the "Trash" row under the tree is selected.
    pub fn is_trash_row_selected(&self) -> bool {
        self.on_trash_row
    }

    /// Whether the "Library" row above the tree (the root) is selected. New documents and
    /// folders then go to the top level: [`selected_folder`](Self::selected_folder) is
    /// `None`.
    pub fn is_root_row_selected(&self) -> bool {
        self.on_root_row
    }

    /// Selects the "Library" row above the tree. This is how the keyboard says "the top
    /// level" when every top-level item is a folder (`up` from the first row and `left`
    /// from a top-level row do it).
    pub fn select_root(&mut self, cx: &mut Context<Self>) {
        self.set_selected(None, cx);
        self.on_root_row = true;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
    }

    /// Selects `item` (or clears the selection with `None`) without expanding anything.
    /// An item that is not in the live tree clears the selection. Use
    /// [`reveal`](Self::reveal) to also make it visible.
    pub fn select(&mut self, item: Option<LibraryItem>, cx: &mut Context<Self>) {
        let tree = self.library.read(cx).tree();
        let item = item.filter(|item| ancestors(&tree, *item).is_some());
        self.set_selected(item, cx);
        self.scroll_to_selected(ScrollStrategy::Center);
    }

    /// Expands the folders that lead to `item`, selects it and scrolls it into view. A
    /// filter that hides the item is cleared. Returns false (and changes nothing) when the
    /// item is not in the live tree.
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
        self.rebuild_rows(cx);
        if self.filter.is_some() && !self.rows.iter().any(|row| row.item == item) {
            self.reset_filter(cx);
        }
        self.set_selected(Some(item), cx);
        self.scroll_to_selected(ScrollStrategy::Center);
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

    /// Tells the panel which document the editor shows, so its row is marked. When the
    /// document changes, the selection follows it: its folders are expanded and its row is
    /// selected and scrolled into view (unless a rename is in progress, or a filter hides
    /// the row).
    pub fn set_active_document(&mut self, id: Option<DocumentId>, cx: &mut Context<Self>) {
        if self.active_document == id {
            return;
        }
        self.active_document = id;
        if let Some(id) = id
            && self.renaming.is_none()
        {
            self.follow(LibraryItem::Document(id), cx);
        }
        cx.notify();
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
        if self.selected.is_some() && self.selected_index().is_none() && self.filter.is_none() {
            self.set_selected(Some(LibraryItem::Folder(folder)), cx);
        }
        cx.notify();
    }

    /// Expands or collapses `folder` and every folder inside it, at any depth (`None`:
    /// every folder of the library). Collapsing what hides the selection moves the
    /// selection to `folder`.
    pub fn set_expanded_all(
        &mut self,
        folder: Option<FolderId>,
        expanded: bool,
        cx: &mut Context<Self>,
    ) {
        fn collect(nodes: &[innerword_store::FolderNode], out: &mut Vec<FolderId>) {
            for node in nodes {
                out.push(node.folder.id);
                collect(&node.folders, out);
            }
        }
        let tree = self.library.read(cx).tree();
        let mut folders = Vec::new();
        match folder {
            Some(id) => {
                let Some(node) = tree.find_folder(id) else {
                    return;
                };
                folders.push(id);
                collect(&node.folders, &mut folders);
            }
            None => collect(&tree.folders, &mut folders),
        }
        let before = self.expanded.len();
        if expanded {
            self.expanded.extend(folders.iter().copied());
        } else {
            for id in &folders {
                self.expanded.remove(id);
            }
        }
        if self.expanded.len() == before {
            return;
        }
        self.persist_expanded(cx);
        self.rebuild_rows(cx);
        if self.selected.is_some() && self.selected_index().is_none() && self.filter.is_none() {
            self.set_selected(folder.map(LibraryItem::Folder), cx);
            if folder.is_none() {
                self.on_root_row = true;
            }
        }
        cx.notify();
    }

    /// The rows of the tree currently visible, top to bottom: collapsed folders hide their
    /// contents, and a filter hides everything that does not match.
    pub fn visible_items(&self) -> Vec<LibraryItem> {
        self.rows.iter().map(|row| row.item).collect()
    }

    // ----- row detail -----------------------------------------------------------------

    /// What the muted detail on document rows shows.
    pub fn row_detail(&self) -> RowDetail {
        self.detail
    }

    /// Sets what the muted detail on document rows shows, and stores the choice.
    pub fn set_row_detail(&mut self, detail: RowDetail, cx: &mut Context<Self>) {
        if self.detail == detail {
            return;
        }
        self.detail = detail;
        // A failed write only loses the choice across restarts.
        let _ = self.library.update(cx, |library, cx| {
            library.set_setting(ROW_DETAIL_SETTING, &detail, cx)
        });
        cx.notify();
    }

    /// The icon drawn on the row of `item`, in the tree as it is shown, else in the trash:
    /// a folder, a text, or the globe of a page. `None` when the item has no row.
    pub fn item_icon(&self, item: LibraryItem) -> Option<IconName> {
        if let Some(row) = self.rows.iter().find(|row| row.item == item) {
            return Some(drag::item_icon(row.item, row.page, row.expanded));
        }
        let row = self.trash_rows.iter().find(|row| row.item == item)?;
        Some(drag::item_icon(row.item, row.page, false))
    }

    /// The muted detail drawn at the right of a visible row: for a document its edit time
    /// or word count ([`RowDetail`]), for a folder the number of documents in it (nothing
    /// when it is empty). `None` when the item has no visible row.
    pub fn item_detail(&self, item: LibraryItem, cx: &App) -> Option<String> {
        let row = self.rows.iter().find(|row| row.item == item)?;
        let now = self.library.read(cx).clock().now_ms();
        self.detail_text(row, now)
    }

    fn detail_text(&self, row: &Row, now_ms: i64) -> Option<String> {
        match row.item {
            LibraryItem::Document(_) => Some(match self.detail {
                RowDetail::Edited => rows::relative_time(now_ms, row.updated_at),
                RowDetail::Words => rows::grouped(row.word_count),
            }),
            LibraryItem::Folder(_) => {
                (row.document_count > 0).then(|| rows::grouped(row.document_count as u64))
            }
        }
    }

    // ----- create, rename -------------------------------------------------------------

    /// Creates an empty document in the [selected folder](Self::selected_folder) and starts
    /// renaming it in place. Once it has a name the panel emits
    /// [`LibraryPanelEvent::OpenDocument`] for it (with `focus_editor: false`). Returns
    /// `None` when no content was set with
    /// [`set_new_document_content`](Self::set_new_document_content) or the store failed
    /// (then a toast says why).
    pub fn new_document(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<DocumentId> {
        let content = self.new_document_content.clone()?;
        let folder = self.selected_folder();
        let created = self.library.update(cx, |library, cx| {
            library.create_document(
                folder,
                &DocumentContent {
                    content: &content,
                    title: "",
                    plain_text: "",
                    word_count: 0,
                    char_count: 0,
                },
                cx,
            )
        });
        match created {
            Ok(meta) => {
                self.begin_rename(LibraryItem::Document(meta.id), window, cx);
                self.rename_opens = true;
                self.rename_is_naming = true;
                Some(meta.id)
            }
            Err(err) => {
                self.report_error(&err.to_string(), cx);
                None
            }
        }
    }

    /// Creates a folder named "New folder" in the [selected folder](Self::selected_folder)
    /// and starts renaming it in place.
    pub fn new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<FolderId> {
        let parent = self.selected_folder();
        let created = self.library.update(cx, |library, cx| {
            library.create_folder(parent, NEW_FOLDER_NAME, cx)
        });
        match created {
            Ok(folder) => {
                self.begin_rename(LibraryItem::Folder(folder.id), window, cx);
                self.rename_is_naming = true;
                Some(folder.id)
            }
            Err(err) => {
                self.report_error(&err.to_string(), cx);
                None
            }
        }
    }

    /// Starts renaming `item` in place: shows the tree, reveals and selects the item, and
    /// puts a focused text field with the current name (all selected) in its row. `enter`
    /// commits, `escape` cancels, and so does moving focus away (it commits). The app calls
    /// this right after creating a folder so the user can type its name. Returns false when
    /// the item is not in the live tree.
    ///
    /// While the name is being typed its row stays on screen: the list does not scroll it
    /// away. When the panel itself is not on screen, see
    /// [`set_reveal_handler`](Self::set_reveal_handler).
    pub fn begin_rename(
        &mut self,
        item: LibraryItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.renaming.is_some() {
            self.finish_rename(true, cx);
        }
        if !self.reveal(item, cx) {
            return false;
        }
        self.view = View::Tree;
        let name = self
            .rows
            .iter()
            .find(|row| row.item == item)
            .filter(|row| !row.untitled)
            .map(|row| row.label.to_string())
            .unwrap_or_default();
        let field = self.ensure_rename_field(window, cx);
        field.update(cx, |field, cx| {
            field.set_text(name, cx);
            field.select_all(cx);
        });
        self.renaming = Some(item);
        self.rename_opens = false;
        self.rename_is_naming = false;
        self.focus_inside(&field.focus_handle(cx), window, cx);
        cx.notify();
        true
    }

    /// The item being renamed in place, if any.
    pub fn renaming(&self) -> Option<LibraryItem> {
        self.renaming
    }

    /// The text field of the rename in progress (for tests and automation).
    pub fn rename_field(&self) -> Option<&Entity<TextField>> {
        self.renaming.and(self.rename_field.as_ref())
    }

    /// Ends the rename in progress and stores the typed name. An empty or unchanged name
    /// changes nothing.
    pub fn commit_rename(&mut self, cx: &mut Context<Self>) {
        self.finish_rename(true, cx);
    }

    /// Leaves rename mode without changing the name.
    pub fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        self.finish_rename(false, cx);
    }

    fn finish_rename(&mut self, commit: bool, cx: &mut Context<Self>) {
        let Some(item) = self.renaming.take() else {
            return;
        };
        let opens = std::mem::take(&mut self.rename_opens);
        let naming = std::mem::take(&mut self.rename_is_naming);
        cx.notify();
        if !commit {
            return;
        }
        let name = self
            .rename_field
            .as_ref()
            .map(|field| field.read(cx).text().trim().to_string())
            .unwrap_or_default();
        // From the tree, not from the rows: the row may be hidden by now.
        let Some(current) = stored_name(&self.library.read(cx).tree(), item) else {
            return;
        };
        if !name.is_empty() && name != current {
            let renamed = self.library.update(cx, |library, cx| match item {
                LibraryItem::Folder(id) => library.rename_folder(id, &name, cx),
                LibraryItem::Document(id) => library.rename_document(id, &name, cx),
            });
            match renamed {
                Ok(()) => {
                    if !naming {
                        self.remember_rename(item, current);
                    }
                    self.rebuild_rows(cx);
                }
                Err(err) => self.report_error(&err.to_string(), cx),
            }
        }
        if opens && let LibraryItem::Document(id) = item {
            cx.emit(LibraryPanelEvent::OpenDocument {
                id,
                focus_editor: false,
            });
        }
    }

    fn ensure_rename_field(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextField> {
        if let Some(field) = &self.rename_field {
            return field.clone();
        }
        let field = cx.new(|cx| {
            TextField::new(window, cx)
                .with_appearance(FieldAppearance::Inline)
                .with_placeholder(rows::UNTITLED)
                .consume_escape()
        });
        let subscription = cx.subscribe_in(
            &field,
            window,
            |this: &mut Self, field, event: &TextFieldEvent, window, cx| match event {
                TextFieldEvent::Submitted | TextFieldEvent::Cancelled => {
                    if this.renaming.is_some() {
                        this.finish_rename(matches!(event, TextFieldEvent::Submitted), cx);
                        if field.focus_handle(cx).is_focused(window) {
                            window.focus(&this.focus_handle);
                        }
                    }
                }
                // Focus moved elsewhere (a click on another row, another pane): keep the
                // typed name, as Finder does.
                TextFieldEvent::Blurred => this.finish_rename(true, cx),
                _ => {}
            },
        );
        self._subscriptions.push(subscription);
        self.rename_field = Some(field.clone());
        field
    }

    // ----- duplicate, move, reorder ---------------------------------------------------

    /// Duplicates the selected item (see [`duplicate_item`](Self::duplicate_item)).
    /// Returns the copy when a document was duplicated, `None` otherwise (also when a
    /// folder was duplicated).
    pub fn duplicate_selected(&mut self, cx: &mut Context<Self>) -> Option<DocumentId> {
        match self.duplicate_item(self.selected?, cx)? {
            LibraryItem::Document(id) => Some(id),
            LibraryItem::Folder(_) => None,
        }
    }

    /// Duplicates `item` and selects the copy, which lands right after the original and is
    /// called "`<name>` copy". A folder is copied with everything in it, at any depth (a
    /// toast says how much). Returns the copy, or `None` when the store refused (a toast
    /// says why).
    pub fn duplicate_item(
        &mut self,
        item: LibraryItem,
        cx: &mut Context<Self>,
    ) -> Option<LibraryItem> {
        let copied = self.library.update(cx, |library, cx| match item {
            LibraryItem::Document(id) => library
                .duplicate_document(id, cx)
                .map(|meta| LibraryItem::Document(meta.id)),
            LibraryItem::Folder(id) => library
                .duplicate_folder(id, cx)
                .map(|folder| LibraryItem::Folder(folder.id)),
        });
        match copied {
            Ok(copy) => {
                self.reveal(copy, cx);
                if let LibraryItem::Folder(id) = copy {
                    let name = self.name_of(item, cx);
                    let contents = self
                        .folder_contents(id, cx)
                        .map(|contents| format!(" with {contents}"))
                        .unwrap_or_default();
                    show_toast(Toast::new(format!("Duplicated “{name}”{contents}.")), cx);
                }
                Some(copy)
            }
            Err(err) => {
                self.report_error(&err.to_string(), cx);
                None
            }
        }
    }

    /// Opens the folder picker for the selected item: every folder it may move into, and
    /// "Library root". Choosing one moves the item there. Returns false when nothing is
    /// selected.
    pub fn open_move_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(item) = self.selected else {
            return false;
        };
        let tree = self.library.read(cx).tree();
        let Some(path) = ancestors(&tree, item) else {
            return false;
        };
        let label: SharedString = item_name(&tree, item).unwrap_or_default().into();
        let delegate =
            MoveDelegate::new(cx.weak_entity(), item, label, path.last().copied(), &tree);
        let picker = cx.new(|cx| Picker::modal(delegate, window, cx).width(px(480.)));
        self.modal_layer
            .update(cx, |layer, cx| layer.show(picker, window, cx));
        true
    }

    /// Moves `item` into `folder` (`None` is the library root), at the end, then reveals
    /// it. Moving it to where it already is does nothing. [`undo`](Self::undo) puts it back
    /// where it was.
    pub fn move_item(
        &mut self,
        item: LibraryItem,
        folder: Option<FolderId>,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let from = place_of(&self.library.read(cx).tree(), item);
        if from.map(|(parent, _)| parent) == Some(folder) {
            return Ok(());
        }
        self.library.update(cx, |library, cx| match item {
            LibraryItem::Folder(id) => library.move_folder(id, folder, None, cx),
            LibraryItem::Document(id) => library.move_document(id, folder, None, cx),
        })?;
        if let Some(from) = from {
            self.remember_move(item, from);
        }
        self.reveal(item, cx);
        Ok(())
    }

    /// [`move_item`](Self::move_item) with a toast that says where the item went, or why
    /// it could not move.
    fn move_item_and_report(
        &mut self,
        item: LibraryItem,
        folder: Option<FolderId>,
        cx: &mut Context<Self>,
    ) {
        if self.parent_of(item, cx) == Some(folder) {
            return;
        }
        let name = self.name_of(item, cx);
        match self.move_item(item, folder, cx) {
            Ok(()) => {
                let place = self.place_label(folder, cx);
                let toast = Toast::new(format!("Moved “{name}” to {place}."));
                show_toast(self.with_undo_button(toast, cx), cx);
            }
            Err(err) => self.report_error(&err.to_string(), cx),
        }
    }

    /// Gives a toast about a change of the panel a button that takes the change back (what
    /// `cmd-z` in the list does), so that a wrong drop or a wrong choice in the folder
    /// picker can be undone with the mouse.
    pub(super) fn with_undo_button(&self, toast: Toast, cx: &mut Context<Self>) -> Toast {
        let panel = cx.weak_entity();
        toast.on_action("Undo", move |_, cx| {
            panel.update(cx, |panel, cx| panel.undo(cx)).ok();
        })
    }

    /// Moves the selected item `delta` places among its siblings (negative is up). Folders
    /// stay among folders and documents among documents. Returns whether it moved.
    pub fn reorder_selected(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        let Some(item) = self.selected else {
            return false;
        };
        // Under a filter the siblings are not all on screen: the move would be invisible.
        if self.filter.is_some() || self.view != View::Tree {
            return false;
        }
        let tree = self.library.read(cx).tree();
        let Some(path) = ancestors(&tree, item) else {
            return false;
        };
        let (folders, documents) = match path.last().and_then(|id| tree.find_folder(*id)) {
            Some(node) => (&node.folders, &node.documents),
            None => (&tree.folders, &tree.documents),
        };
        let (index, count) = match item {
            LibraryItem::Folder(id) => (
                folders.iter().position(|node| node.folder.id == id),
                folders.len(),
            ),
            LibraryItem::Document(id) => (
                documents.iter().position(|document| document.id == id),
                documents.len(),
            ),
        };
        let Some(index) = index else {
            return false;
        };
        let target = index as isize + delta;
        if target < 0 || target >= count as isize || delta == 0 {
            return false;
        }
        let target = target as usize;
        let moved = self.library.update(cx, |library, cx| match item {
            LibraryItem::Folder(id) => library.reorder_folder(id, target, cx),
            LibraryItem::Document(id) => library.reorder_document(id, target, cx),
        });
        match moved {
            Ok(()) => {
                self.remember_reorder(item, (path.last().copied(), index));
                self.rebuild_rows(cx);
                self.scroll_to_selected(if delta < 0 {
                    ScrollStrategy::Top
                } else {
                    ScrollStrategy::Bottom
                });
                cx.notify();
                true
            }
            Err(err) => {
                self.report_error(&err.to_string(), cx);
                false
            }
        }
    }

    // ----- trash ----------------------------------------------------------------------

    /// Moves the selected item to the trash and shows a toast that names it and has an
    /// "Undo" button (with the key of the list's undo when the keyboard is in the panel).
    /// that undoes it. Returns whether something was trashed.
    pub fn trash_selected(&mut self, cx: &mut Context<Self>) -> bool {
        match self.selected {
            Some(item) if self.view == View::Tree => self.trash_item(item, cx),
            _ => false,
        }
    }

    /// Moves `item` to the trash, with the same toast as
    /// [`trash_selected`](Self::trash_selected).
    pub fn trash_item(&mut self, item: LibraryItem, cx: &mut Context<Self>) -> bool {
        let mut name = self.name_of(item, cx);
        // A folder takes its contents along: say how much went with it, in few words.
        let contents = match item {
            LibraryItem::Folder(id) => self.folder_contents_brief(id, cx),
            LibraryItem::Document(_) => None,
        }
        .map(|contents| format!(" and {contents}"))
        .unwrap_or_default();
        if !contents.is_empty() {
            name = short(&name, NAME_IN_MESSAGE - 8);
        }
        let in_panel = self.keyboard_inside;
        let trashed = self.library.update(cx, |library, cx| match item {
            LibraryItem::Folder(id) => library.trash_folder(id, cx),
            LibraryItem::Document(id) => library.trash_document(id, cx),
        });
        match trashed {
            Ok(()) => {
                // Repair the selection now; the library's events arrive after this update.
                self.tree_changed(cx);
                let toast = Toast::new(format!("Moved “{name}”{contents} to the trash."))
                    .duration(Duration::from_secs(6));
                // The toast has a button that takes it back. With the keyboard in the
                // panel it is the list's own undo and shows its key. With the keyboard
                // elsewhere (the command came from the palette while writing) that key
                // would undo typing: the button shows no key and restores this item.
                let toast = if in_panel {
                    toast.action("Undo", UndoTrash)
                } else {
                    let panel = cx.weak_entity();
                    toast.on_action("Undo", move |_, cx| {
                        panel
                            .update(cx, |panel, cx| panel.restore_item(item, cx))
                            .ok();
                    })
                };
                show_toast(toast, cx);
                true
            }
            Err(err) => {
                self.report_error(&err.to_string(), cx);
                false
            }
        }
    }

    /// Whether the panel shows the trash instead of the tree.
    pub fn is_showing_trash(&self) -> bool {
        self.view == View::Trash
    }

    /// Shows the trash view and focuses the panel.
    pub fn show_trash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.renaming.is_some() {
            self.finish_rename(true, cx);
        }
        if self.view != View::Trash {
            // A fresh look at the trash starts at the most recently trashed item.
            self.view = View::Trash;
            self.trash_selected = None;
            self.trash_scroll
                .scroll_to_item_strict(0, ScrollStrategy::Top);
        }
        self.refresh_trash(cx);
        let handle = self.focus_handle.clone();
        self.focus_inside(&handle, window, cx);
        cx.notify();
    }

    /// Goes back from the trash view to the tree.
    pub fn show_tree(&mut self, cx: &mut Context<Self>) {
        if self.view != View::Tree {
            self.view = View::Tree;
            self.scroll_to_selected(ScrollStrategy::Center);
            cx.notify();
        }
    }

    /// The top-level entries of the trash, most recently trashed first (cached; refreshed
    /// when the trash changes).
    pub fn trash_items(&self) -> Vec<LibraryItem> {
        self.trash_rows.iter().map(|row| row.item).collect()
    }

    /// Where a trashed item came from, as shown in the trash view: "Novel / Part one" or
    /// "Library root". `None` when the item is not a top-level entry of the trash.
    pub fn trash_location(&self, item: LibraryItem) -> Option<String> {
        self.trash_rows
            .iter()
            .find(|row| row.item == item)
            .map(|row| row.location.to_string())
    }

    /// The selected row of the trash view.
    pub fn selected_trash_item(&self) -> Option<LibraryItem> {
        self.trash_selected
    }

    /// Restores the selected item of the trash view to where it came from (the library
    /// root if that folder is gone) and reveals it in the tree. The trash view stays open.
    pub fn restore_selected(&mut self, cx: &mut Context<Self>) -> bool {
        match self.trash_selected {
            Some(item) if self.view == View::Trash => self.restore_item(item, cx).is_some(),
            _ => false,
        }
    }

    /// Restores a trashed item, reveals it in the tree and says where it went.
    pub(super) fn restore_item(&mut self, item: LibraryItem, cx: &mut Context<Self>) -> Option<()> {
        let restored = self.library.update(cx, |library, cx| match item {
            LibraryItem::Folder(id) => library.restore_folder(id, cx).map(|_| ()),
            LibraryItem::Document(id) => library.restore_document(id, cx).map(|_| ()),
        });
        match restored {
            Ok(()) => {
                self.forget_trashed(item);
                self.reveal(item, cx);
                self.refresh_trash(cx);
                let name = self.name_of(item, cx);
                let place = self.place_label(self.parent_of(item, cx).flatten(), cx);
                show_toast(Toast::new(format!("Restored “{name}” to {place}.")), cx);
                cx.notify();
                Some(())
            }
            // Not in the trash any more (restored or deleted elsewhere): nothing to say.
            Err(innerword_store::StoreError::NotInTrash { .. })
            | Err(innerword_store::StoreError::NotFound { .. }) => None,
            Err(err) => {
                self.report_error(&err.to_string(), cx);
                None
            }
        }
    }

    /// Asks whether to delete the selected item of the trash view for good, and does it on
    /// confirmation. Returns whether the question was asked.
    pub fn delete_selected_forever(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(row) = self.selected_trash_row().cloned() else {
            return false;
        };
        if self.view != View::Trash {
            return false;
        }
        let message = match &row.contents {
            Some(contents) => {
                format!("The folder goes with what is in it: {contents}. This cannot be undone.")
            }
            None => "This cannot be undone.".to_string(),
        };
        let panel = cx.weak_entity();
        let item = row.item;
        let dialog = cx.new(|cx| {
            ConfirmDialog::new(
                format!("Delete “{}” forever?", short(&row.label, 40)),
                message,
                cx,
            )
            .confirm_label("Delete forever")
            .danger()
            .on_confirm(move |_, cx| {
                panel.update(cx, |panel, cx| panel.purge(item, cx)).ok();
            })
        });
        ConfirmDialog::show(dialog, &self.modal_layer, window, cx);
        true
    }

    /// Asks whether to delete everything in the trash for good, and does it on
    /// confirmation. With an empty trash it only says so. Returns whether the question was
    /// asked.
    pub fn empty_trash(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.refresh_trash(cx);
        if self.trash_rows.is_empty() {
            show_toast(Toast::new("The trash is already empty."), cx);
            return false;
        }
        let panel = cx.weak_entity();
        let count = self.trash_rows.len() as u64;
        let dialog = cx.new(|cx| {
            ConfirmDialog::new(
                "Empty the trash?",
                format!(
                    "{} will be deleted forever. This cannot be undone.",
                    plural(count, "item")
                ),
                cx,
            )
            .confirm_label("Empty trash")
            .danger()
            .on_confirm(move |_, cx| {
                panel.update(cx, |panel, cx| panel.purge_all(cx)).ok();
            })
        });
        ConfirmDialog::show(dialog, &self.modal_layer, window, cx);
        true
    }

    fn purge(&mut self, item: LibraryItem, cx: &mut Context<Self>) {
        let name = self
            .trash_rows
            .iter()
            .find(|row| row.item == item)
            .map(|row| short(&row.label, NAME_IN_MESSAGE))
            .unwrap_or_default();
        let purged = self.library.update(cx, |library, cx| match item {
            LibraryItem::Folder(id) => library.purge_folder(id, cx),
            LibraryItem::Document(id) => library.purge_document(id, cx),
        });
        match purged {
            Ok(()) => {
                self.refresh_trash(cx);
                show_toast(Toast::new(format!("Deleted “{name}”.")), cx);
            }
            Err(err) => self.report_error(&err.to_string(), cx),
        }
    }

    fn purge_all(&mut self, cx: &mut Context<Self>) {
        let emptied = self
            .library
            .update(cx, |library, cx| library.empty_trash(cx));
        match emptied {
            Ok(emptied) => {
                self.refresh_trash(cx);
                let mut parts = Vec::new();
                if emptied.documents > 0 {
                    parts.push(plural(emptied.documents, "document"));
                }
                if emptied.folders > 0 {
                    parts.push(plural(emptied.folders, "folder"));
                }
                let message = if parts.is_empty() {
                    "The trash is empty.".to_string()
                } else {
                    format!("Deleted {}.", parts.join(" and "))
                };
                show_toast(Toast::new(message), cx);
            }
            Err(err) => self.report_error(&err.to_string(), cx),
        }
    }

    // ----- filter ---------------------------------------------------------------------

    /// The text of the name filter (empty when the tree is not filtered).
    pub fn filter(&self) -> &str {
        &self.filter_text
    }

    /// Filters the tree by name: every word of `text` must occur in an item's name,
    /// ignoring case. The rows are the matches, everything inside a matching folder, and
    /// the folders that lead to them. The first match is selected. An empty text clears
    /// the filter.
    pub fn set_filter(&mut self, text: &str, cx: &mut Context<Self>) {
        if let Some(field) = &self.filter_field
            && field.read(cx).text() != text
        {
            field.update(cx, |field, cx| field.set_text(text, cx));
        }
        self.apply_filter(text.to_string(), cx);
    }

    /// Clears the filter. The selection stays where it is and is revealed in the full
    /// tree; when the filter matched nothing, the selection goes back to what it was before
    /// the filter.
    pub fn clear_filter(&mut self, cx: &mut Context<Self>) {
        if self.filter_text.is_empty() {
            return;
        }
        self.reset_filter(cx);
        self.reveal_after_filter(cx);
        cx.notify();
    }

    /// The filter is off again: show the selection in the full tree, or go back to the
    /// selection from before the filter if nothing is selected.
    fn reveal_after_filter(&mut self, cx: &mut Context<Self>) {
        let before = self.selection_before_filter.take();
        if let Some(item) = self.selected.or(before) {
            self.reveal(item, cx);
        }
    }

    /// Shows the tree and puts the keyboard focus in the filter field.
    pub fn focus_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.renaming.is_some() {
            self.finish_rename(true, cx);
        }
        self.view = View::Tree;
        let field = self.ensure_filter_field(window, cx);
        field.update(cx, |field, cx| field.select_all(cx));
        self.focus_inside(&field.focus_handle(cx), window, cx);
        cx.notify();
    }

    /// The filter's text field, once the panel was rendered or the filter was focused
    /// (for tests and automation).
    pub fn filter_field(&self) -> Option<&Entity<TextField>> {
        self.filter_field.as_ref()
    }

    fn apply_filter(&mut self, text: String, cx: &mut Context<Self>) {
        if text == self.filter_text {
            return;
        }
        // The row being renamed may not survive the filter: keep the typed name first.
        if self.renaming.is_some() {
            self.finish_rename(true, cx);
        }
        if text.trim().is_empty() {
            self.filter_text = text;
            if self.filter.take().is_some() {
                self.rebuild_rows(cx);
                self.reveal_after_filter(cx);
            }
            cx.notify();
            return;
        }
        if self.filter.is_none() {
            self.selection_before_filter = self.selected;
        }
        self.filter = Filter::new(&text);
        self.filter_text = text;
        self.rebuild_rows(cx);
        // Like a picker: the best candidate is the first row that matches by name.
        let first = self
            .rows
            .iter()
            .find(|row| !row.positions.is_empty())
            .or(self.rows.first())
            .map(|row| row.item);
        self.set_selected(first, cx);
        self.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
        self.scroll_to_selected(ScrollStrategy::Center);
        cx.notify();
    }

    /// Drops the filter and its text without touching the selection.
    fn reset_filter(&mut self, cx: &mut Context<Self>) {
        self.filter = None;
        self.filter_text.clear();
        if let Some(field) = &self.filter_field {
            field.update(cx, |field, cx| field.set_text("", cx));
        }
        self.rebuild_rows(cx);
    }

    fn ensure_filter_field(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextField> {
        if let Some(field) = &self.filter_field {
            return field.clone();
        }
        let text = self.filter_text.clone();
        let field = cx.new(|cx| {
            let mut field = TextField::new(window, cx)
                .with_appearance(FieldAppearance::Borderless)
                .with_placeholder("Filter")
                .with_text(text)
                .consume_escape();
            // up / down / home / end keep moving the selection in the tree.
            field.set_list_navigation(true);
            field
        });
        let subscription = cx.subscribe_in(
            &field,
            window,
            |this: &mut Self, field, event: &TextFieldEvent, window, cx| match event {
                TextFieldEvent::Changed => {
                    let text = field.read(cx).text().to_string();
                    this.apply_filter(text, cx);
                }
                // enter: open the selected document, or continue in the tree on the selected
                // folder. With no match there is nothing to continue with: the keyboard
                // stays in the field.
                TextFieldEvent::Submitted => match this.selected {
                    Some(LibraryItem::Document(id)) => cx.emit(LibraryPanelEvent::OpenDocument {
                        id,
                        focus_editor: true,
                    }),
                    Some(LibraryItem::Folder(_)) => window.focus(&this.focus_handle),
                    None => {}
                },
                TextFieldEvent::Cancelled => {
                    this.clear_filter(cx);
                    window.focus(&this.focus_handle);
                }
                _ => {}
            },
        );
        self._subscriptions.push(subscription);
        self.filter_field = Some(field.clone());
        field
    }

    // ----- internals ------------------------------------------------------------------

    fn selected_index(&self) -> Option<usize> {
        let selected = self.selected?;
        self.rows.iter().position(|row| row.item == selected)
    }

    fn selected_row(&self) -> Option<&Row> {
        self.selected_index().map(|ix| &self.rows[ix])
    }

    fn selected_trash_index(&self) -> Option<usize> {
        let selected = self.trash_selected?;
        self.trash_rows.iter().position(|row| row.item == selected)
    }

    fn selected_trash_row(&self) -> Option<&TrashRow> {
        self.selected_trash_index().map(|ix| &self.trash_rows[ix])
    }

    fn scroll_to_selected(&self, strategy: ScrollStrategy) {
        if let Some(ix) = self.selected_index() {
            self.scroll.scroll_to_item(ix, strategy);
        }
    }

    /// Changes the tree's selection and schedules storing it.
    fn set_selected(&mut self, item: Option<LibraryItem>, cx: &mut Context<Self>) {
        self.on_trash_row = false;
        self.on_root_row = false;
        if self.selected != item {
            self.selected = item;
            self.schedule_persist_selection(cx);
        }
        cx.notify();
    }

    fn select_trash_row(&mut self, cx: &mut Context<Self>) {
        self.set_selected(None, cx);
        self.on_trash_row = true;
    }

    fn select_index(&mut self, ix: usize, strategy: ScrollStrategy, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.get(ix) {
            self.set_selected(Some(row.item), cx);
            self.scroll.scroll_to_item(ix, strategy);
        }
    }

    fn select_trash_index(&mut self, ix: usize, strategy: ScrollStrategy, cx: &mut Context<Self>) {
        if let Some(row) = self.trash_rows.get(ix) {
            self.trash_selected = Some(row.item);
            self.trash_scroll.scroll_to_item(ix, strategy);
            cx.notify();
        }
    }

    /// Makes the selection follow `item` without disturbing a filter.
    fn follow(&mut self, item: LibraryItem, cx: &mut Context<Self>) {
        if self.filter.is_none() {
            self.reveal(item, cx);
        } else if self.rows.iter().any(|row| row.item == item) {
            self.set_selected(Some(item), cx);
            self.scroll_to_selected(ScrollStrategy::Center);
        }
    }

    /// The folder that holds a live item: `Some(None)` at the root, `None` when the item
    /// is not in the tree.
    fn parent_of(&self, item: LibraryItem, cx: &App) -> Option<Option<FolderId>> {
        let tree = self.library.read(cx).tree();
        ancestors(&tree, item).map(|path| path.last().copied())
    }

    /// The name of a live item as it appears in a message: cut to [`NAME_IN_MESSAGE`]
    /// characters, so a long title cannot push the end of a one-line toast out of sight.
    pub(super) fn name_of(&self, item: LibraryItem, cx: &App) -> String {
        let tree = self.library.read(cx).tree();
        short(&item_name(&tree, item).unwrap_or_default(), NAME_IN_MESSAGE)
    }

    /// A folder as a place in a sentence: "Novel / Part one", or "the library root". A
    /// long path keeps its end ("… / Part one").
    pub(super) fn place_label(&self, folder: Option<FolderId>, cx: &App) -> String {
        let path = self.library.read(cx).folder_path(folder);
        if path.is_empty() {
            format!("the {}", ROOT_LABEL.to_lowercase())
        } else {
            short_path(&path, PLACE_IN_MESSAGE)
        }
    }

    pub(super) fn report_error(&self, message: &str, cx: &mut Context<Self>) {
        show_toast(Toast::new(message.to_string()).error(), cx);
    }

    fn persist_expanded(&mut self, cx: &mut Context<Self>) {
        let mut ids: Vec<FolderId> = self.expanded.iter().copied().collect();
        ids.sort();
        // A failed write only loses the expansion state across restarts.
        let _ = self.library.update(cx, |library, cx| {
            library.set_setting(EXPANDED_FOLDERS_SETTING, &ids, cx)
        });
    }

    /// Stores the selection a moment later, so holding an arrow key does not write to the
    /// database on every step.
    fn schedule_persist_selection(&mut self, cx: &mut Context<Self>) {
        self.selection_dirty = true;
        self.persist_selection = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(SELECTION_PERSIST_DELAY)
                .await;
            this.update(cx, |this, cx| this.persist_selection_now(cx))
                .ok();
        }));
    }

    fn persist_selection_now(&mut self, cx: &mut Context<Self>) {
        if !std::mem::take(&mut self.selection_dirty) {
            return;
        }
        let selected = self.selected;
        // A failed write only loses the selection across restarts.
        let _ = self.library.update(cx, |library, cx| {
            library.set_setting(SELECTED_ITEM_SETTING, &selected, cx)
        });
    }

    fn rebuild_rows(&mut self, cx: &mut Context<Self>) {
        let tree = self.library.read(cx).tree();
        self.rows = build_rows(&tree, &self.expanded, self.filter.as_ref());
    }

    /// Re-reads the trash listing, forgets undo entries that are no longer in the trash and
    /// keeps the trash view's selection on a row.
    fn refresh_trash(&mut self, cx: &mut Context<Self>) {
        let old_index = self.selected_trash_index();
        self.trash_rows = build_trash_rows(self.library.read(cx));
        let in_trash: HashSet<LibraryItem> = self.trash_rows.iter().map(|row| row.item).collect();
        self.forget_trashed_except(|item| in_trash.contains(&item));
        if self.selected_trash_index().is_none() {
            let last = self.trash_rows.len().saturating_sub(1);
            self.trash_selected = self
                .trash_rows
                .get(old_index.unwrap_or(0).min(last))
                .map(|row| row.item);
        }
        cx.notify();
    }

    fn library_event(&mut self, event: &LibraryEvent, cx: &mut Context<Self>) {
        match event {
            LibraryEvent::TreeChanged => self.tree_changed(cx),
            // Word counts and edit times changed; names and structure did not.
            LibraryEvent::DocumentSaved {
                title_changed: false,
                id,
            } => self.document_saved(*id, cx),
            LibraryEvent::DocumentTrashed(id) => self.remember_trashed(LibraryItem::Document(*id)),
            LibraryEvent::FolderTrashed { id, .. } => {
                self.remember_trashed(LibraryItem::Folder(*id))
            }
            LibraryEvent::TrashChanged => self.refresh_trash(cx),
            _ => {}
        }
    }

    /// A document was saved under the same title: its row shows a new edit time and word
    /// count, and nothing else in the list changed. This runs after every autosave, so it
    /// touches one row instead of rebuilding them all.
    fn document_saved(&mut self, id: DocumentId, cx: &mut Context<Self>) {
        let Some(ix) = self
            .rows
            .iter()
            .position(|row| row.item == LibraryItem::Document(id))
        else {
            return;
        };
        let tree = self.library.read(cx).tree();
        let saved = tree
            .documents_in(self.rows[ix].parent)
            .and_then(|documents| documents.iter().find(|document| document.id == id));
        match saved {
            Some(meta) => {
                self.rows[ix].updated_at = meta.updated_at;
                self.rows[ix].word_count = meta.word_count;
            }
            None => self.rebuild_rows(cx),
        }
        cx.notify();
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
            let item = old_index
                .and_then(|ix| self.rows.get(ix.min(self.rows.len().saturating_sub(1))))
                .map(|row| row.item);
            self.set_selected(item, cx);
        }
        if self
            .renaming
            .is_some_and(|item| ancestors(&tree, item).is_none())
        {
            self.renaming = None;
            self.rename_opens = false;
            self.rename_is_naming = false;
        }
        if self
            .active_document
            .is_some_and(|id| ancestors(&tree, LibraryItem::Document(id)).is_none())
        {
            self.active_document = None;
        }
        cx.notify();
    }

    /// The panel got the keyboard focus: make sure a row is selected, so focus is visible.
    fn focused(&mut self, cx: &mut Context<Self>) {
        if self.view == View::Tree
            && self.selected.is_none()
            && !self.on_trash_row
            && !self.on_root_row
        {
            let item = self
                .active_document
                .map(LibraryItem::Document)
                .filter(|item| self.rows.iter().any(|row| row.item == *item))
                .or(self.rows.first().map(|row| row.item));
            if item.is_some() {
                self.set_selected(item, cx);
                self.scroll_to_selected(ScrollStrategy::Center);
            }
        }
    }

    /// What a folder holds, in two words: its documents at any depth ("3 documents"), or its
    /// subfolders when it has no documents. `None` when it is empty or not in the tree.
    fn folder_contents_brief(&self, folder: FolderId, cx: &App) -> Option<String> {
        let tree = self.library.read(cx).tree();
        let node = tree.find_folder(folder)?;
        match (count_documents(node), count_folders(node)) {
            (0, 0) => None,
            (0, folders) => Some(plural(folders as u64, "subfolder")),
            (documents, _) => Some(plural(documents as u64, "document")),
        }
    }

    /// What a folder holds ("3 documents, 1 subfolder"), or `None` when it is empty or not
    /// in the tree.
    fn folder_contents(&self, folder: FolderId, cx: &App) -> Option<String> {
        let tree = self.library.read(cx).tree();
        let node = tree.find_folder(folder)?;
        let (documents, folders) = (count_documents(node), count_folders(node));
        let mut parts = Vec::new();
        if documents > 0 {
            parts.push(plural(documents as u64, "document"));
        }
        if folders > 0 {
            parts.push(plural(folders as u64, "subfolder"));
        }
        (!parts.is_empty()).then(|| parts.join(", "))
    }
}
