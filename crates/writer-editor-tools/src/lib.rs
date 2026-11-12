//! The contextual tools of the editor: the format menu, the insert menu,
//! the link popover, the font picker, find and replace, the context menu
//! of the text, and the format bar that floats over a mouse selection.
//!
//! Everything here works from the keyboard alone and with the mouse alone,
//! and plugs into `writer_editor::Editor` through its public seams (the
//! mutation seam, key interception, popup hosting, highlight layers, the
//! context menu request).
//!
//! # Setup
//!
//! ```ignore
//! // once at startup, after the app's context-free bindings and after
//! // writer_ui::init and writer_editor::init
//! writer_editor_tools::init(cx);
//!
//! // per editor: wrap it, and render the wrapper where the editor would go
//! let editor = cx.new(|cx| Editor::new(document, EditorConfig::full(), window, cx));
//! let tools = cx.new(|cx| EditorTools::new(editor.clone(), window, cx));
//! // in the parent's render
//! div().size_full().child(tools.clone())
//!
//! // command palette
//! let commands = writer_editor::commands().into_iter().chain(writer_editor_tools::commands());
//! ```
//!
//! [`EditorTools`] renders the editor, shows the find bar above or below it
//! ([`EditorTools::set_find_bar_placement`]) and handles the actions in
//! [`actions`] while focus is anywhere inside it. The editor entity stays
//! usable as before (`tools.read(cx).editor()`); focusing the tools view
//! focuses the editor.
//!
//! # The tools
//!
//! | Tool | Keys | Notes |
//! |---|---|---|
//! | Format menu | `cmd-.` | Type to filter, `up` / `down`, `enter` applies and closes, `escape` closes. Inline formatting first with a selection, block options first with a collapsed caret. Rows show the state of the selection (check, or a dash for "in part of it") and the real key binding. |
//! | Insert menu | `/` at the start of a block or after whitespace | Focus stays in the document; the text after the slash filters. `enter` or `tab` applies (slash and query are removed in the same undo step), `escape` closes and keeps the text. Also [`actions::OpenInsertMenu`] for the palette. |
//! | Link popover | `cmd-k` | `enter` applies (inserts the URL as linked text at a collapsed caret outside a link), `cmd-enter` opens the URL in the browser, `cmd-shift-k` removes the link, `escape` cancels. |
//! | Font picker | `cmd-t` | Document default, recent, all families. `enter` applies to the selection or to the text typed next, `cmd-enter` sets the document's body font. |
//! | Find and replace | `cmd-f`, `cmd-alt-f` | `enter` / `shift-enter` and `cmd-g` / `cmd-shift-g` step through the matches, `cmd-alt-c` case, `cmd-alt-w` whole word, `tab` between the fields, `enter` in the replace field replaces the current match, `cmd-enter` replaces all, `cmd-alt-f` in the bar shows or hides the replace row, `escape` (in the bar or in the document) closes and leaves the current match selected. |
//!
//! | Context menu of the text | right-click, `ctrl`-click, `ctrl-enter` | What fits the place: the link under the pointer (open, edit, remove, copy its address); cut, copy, paste; the formatting of the selection (highlight and text colour as submenus); the block (turn into, align, text size, mark a to-do done); insert; find, select all; then the host's items. What the host puts first (spelling suggestions) leads the menu, see [`EditorTools::set_text_menu_leading_extension`]. Every row runs what its key runs and shows that key. See [`EditorTools::open_text_menu`]. |
//!
//! | Format bar | a selection made with the mouse | Comes up by itself a moment after a drag, a double or triple click or a `shift`-click, over the selection: bold, italic, underline, strikethrough, inline code (showing the state of the selection), highlight, text colour, link, "turn into", the host's buttons, and "more" (the format menu). Never takes the keyboard; a button keeps the selection and the bar. Goes on `escape`, a keyboard selection, typing, a press elsewhere. See [`EditorTools::set_format_bar_enabled`] and the `format_bar` module docs. |
//!
//! With the mouse: every row of the menus can be clicked (the row under
//! the pointer is the selected one), the buttons of the link popover and
//! of the find bar have tooltips with their keys, a press outside a popup
//! closes it, and a row of the font picker has a menu of its own
//! (right-click, or the small button the selected row shows) for "Set as
//! Document Font".
//!
//! The editor hosts one popup, so opening a popup tool closes the popup
//! tool that is open. The find bar is not a popup: it stays open, with its
//! query, matches and highlights, while a format menu, link popover, font
//! picker or insert menu is used, and `escape` then closes one thing at a
//! time (the popup first). Opening the find bar closes an open popup tool,
//! because the bar takes the keyboard.
//!
//! With the find bar closed, `cmd-g` / `cmd-shift-g` search for the last
//! query without opening it. For [`FIND_FLASH_DURATION`] after each step
//! the matches are highlighted and a small label over the top right corner
//! of the editor shows the query and "3 of 17" (or "No matches"); then
//! both go away by themselves, or at once with `escape`.
//!
//! While find results are showing, an edit updates the match list from the
//! blocks it changed (`writer_core`'s change report and
//! `Document::find_in_block`); the whole document is searched only when the
//! query or an option changes. Measured by `tests/perf.rs` at 10,000
//! blocks with 40,000 matches, per character typed in the document: debug
//! build 13.2 ms with a search per edit, 3.3 ms now, 0.7 ms with the bar
//! closed; release build 0.33 ms now and 0.11 ms with the bar closed,
//! against 2.2 ms for one search of the whole document. What is left is
//! handing the editor the highlight ranges, and that is skipped when the
//! edit touched no match (0.19 ms in release with a query that is not in
//! the paragraph being typed).
//!
//! # Notes for the host
//!
//! - **Focus.** `EditorTools` is `Focusable` with the editor's handle. A
//!   popup tool with a field (format menu, link popover, font picker) takes
//!   focus; it closes when focus moves elsewhere in the window and gives
//!   focus back to the editor on `escape` or after applying. If the host
//!   opens a modal while such a tool has focus, the tool closes and the
//!   focus handle the modal remembered is gone: fall back to focusing the
//!   tools view when restoring focus fails.
//! - **The context menu** is `writer_ui::ContextMenu`, drawn by the
//!   window's `writer_ui::ContextMenuLayer` (the toast layer has one). The
//!   editor asks for it (`EditorEvent::ContextMenuRequested`) and this
//!   view opens it; add the app's own items with
//!   [`EditorTools::set_text_menu_extension`] ("Add Comment" for a
//!   selection, "Go to Comment" on commented text) and what belongs at
//!   the very top with [`EditorTools::set_text_menu_leading_extension`]
//!   (spelling suggestions on a marked word). The menu never takes
//!   the keyboard: the editor keeps it, and an action item is dispatched
//!   from the editor outwards as its key binding is.
//! - **The format bar** is on by default; connect a setting to
//!   [`EditorTools::set_format_bar_enabled`] (and read
//!   [`EditorTools::format_bar_enabled`]). Add the app's buttons with
//!   [`EditorTools::set_format_bar_extension`], the counterpart of the
//!   text menu's hook: a list of [`FormatBarButton`]s, asked for each time
//!   the bar appears ("Add Comment"). The bar needs nothing mounted; the
//!   menus its buttons drop are `writer_ui::ContextMenu`s and need the
//!   window's `ContextMenuLayer` like every other menu. It is not a
//!   [`ToolKind`] and [`EditorTools::active_tool`] does not report it (it
//!   never has the keyboard): read [`EditorTools::format_bar_visible`],
//!   [`EditorTools::format_bar_items`] and
//!   [`EditorTools::format_bar_bounds`], and press a button from a script
//!   with [`EditorTools::press_format_bar_button`]. `escape` in the text
//!   closes a showing bar before the find bar.
//! - **Key contexts.** The bindings live in `"EditorTools"` (the whole
//!   view), `"FindBar"` and `"LinkPopover"`. Inside the find bar
//!   `cmd-alt-c` toggles case sensitivity; a context-free binding for the
//!   same key (the app's "show comments") must be registered before
//!   [`init`] so that the find bar's binding wins there. `escape` is bound
//!   in `"EditorTools"` to [`actions::CloseTool`], which passes the key on
//!   when nothing is open; it only gets there from the document (text
//!   fields, an open insert menu and Vim mode take `escape` first).
//! - **The insert menu's trigger** is `EditorEvent::TextInput` with the
//!   text `/`: the menu opens for a slash that was typed (on any keyboard
//!   layout, also in Vim's insert mode), never for one that was pasted,
//!   inserted by a command or brought back by undo, and not when the key
//!   typed nothing (Vim's normal mode, a read-only document).
//! - **Text colours.** [`TEXT_COLORS`] are stored in the document as plain
//!   RGB and painted as stored in both themes, so they are balanced
//!   between the two (about 4:1 on each); see the constant.
//! - **Fonts.** The family list and the recently used families are in the
//!   [`FontLibrary`] global: restore them with [`set_recent_fonts`] at
//!   startup and persist [`recent_fonts`] from
//!   `cx.observe_global::<FontLibrary>(..)`.
//! - **State for a status line or automation:**
//!   [`EditorTools::active_tool`], [`EditorTools::menu_rows`],
//!   [`EditorTools::find_status`], [`EditorTools::find_flash_visible`],
//!   [`EditorTools::find_button_tooltip`],
//!   [`EditorTools::link_button_enabled`], [`EditorTools::text_menu`] and
//!   [`EditorTools::text_menu_request`], and [`ToolsEvent`] on open and
//!   close.
//! - **Highlight layers.** Find uses the editor layers [`FIND_LAYER`] and
//!   [`FIND_CURRENT_LAYER`]; other crates should pick other names.
//!
//! # Module map
//!
//! | Module | Contents |
//! |---|---|
//! | `tools` | [`EditorTools`]: state, action handlers, editor events, popup plumbing, render |
//! | `menu` | the popup list shared by the format menu, the insert menu and the font picker |
//! | `format_menu`, `insert_menu`, `link`, `font_picker`, `find` | one tool each, as `impl EditorTools` blocks |
//! | `text_menu` | the context menu of the text: its items for a request, the host's extension, what each row runs |
//! | `format_bar` | the floating format bar: telling a mouse selection from a keyboard one, placing and drawing the bar, its buttons and their menus, the host's buttons |
//! | [`actions`] | actions and key bindings |
//! | `tip` | the tooltip of a button: its name and key |
//! | `util` | today's date, URL clean-up |

pub mod actions;
mod commands;
mod find;
mod font_picker;
mod format_bar;
mod format_menu;
mod insert_menu;
mod link;
mod menu;
mod text_menu;
mod tip;
mod tools;
mod util;

pub use commands::{CATEGORY_FORMAT, CATEGORY_INSERT, CATEGORY_SEARCH, commands};
pub use find::{FIND_BUTTONS, FIND_CURRENT_LAYER, FIND_FLASH_DURATION, FIND_LAYER};
pub use font_picker::{
    FontLibrary, RECENT_FONT_LIMIT, WRITING_FONTS, font_families, recent_fonts, set_font_families,
    set_recent_fonts,
};
pub use format_bar::{FORMAT_BAR_BUTTONS, FORMAT_BAR_DELAY, FormatBarButton, FormatBarItem};
pub use format_menu::TEXT_COLORS;
pub use menu::Mark;
#[doc(hidden)]
pub use menu::selected_row_fill;
pub use tools::{EditorTools, FindBarPlacement, MenuRow, ToolKind, ToolsEvent};
pub use util::{format_date, normalize_url, today_string};

/// Installs the key bindings of the tools. Call once at startup, after the
/// app's own context-free bindings, `writer_ui::init` and
/// `writer_editor::init`.
pub fn init(cx: &mut gpui::App) {
    cx.bind_keys(actions::key_bindings());
}
