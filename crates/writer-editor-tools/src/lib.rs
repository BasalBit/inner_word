//! The contextual tools of the editor: the format menu, the insert menu,
//! the link popover, the font picker, and find and replace.
//!
//! Everything here is driven from the keyboard and plugs into
//! `writer_editor::Editor` through its public seams (the mutation seam,
//! key interception, popup hosting, highlight layers).
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
//! query or an option changes. At 10,000 blocks and 40,000 matches a typed
//! character costs about a quarter of what it did with a search per edit
//! (debug build: 13 ms before, 3.3 ms after, 0.7 ms with the bar closed);
//! what is left is handing the editor the highlight ranges.
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
//!   [`EditorTools::find_button_tooltip`], and [`ToolsEvent`] on open and
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
//! | [`actions`] | actions and key bindings |
//! | `tip` | the tooltip of a button: its name and key |
//! | `util` | today's date, URL clean-up |

pub mod actions;
mod commands;
mod find;
mod font_picker;
mod format_menu;
mod insert_menu;
mod link;
mod menu;
mod tip;
mod tools;
mod util;

pub use commands::{CATEGORY_FORMAT, CATEGORY_INSERT, CATEGORY_SEARCH, commands};
pub use find::{FIND_BUTTONS, FIND_CURRENT_LAYER, FIND_FLASH_DURATION, FIND_LAYER};
pub use font_picker::{
    FontLibrary, RECENT_FONT_LIMIT, font_families, recent_fonts, set_font_families,
    set_recent_fonts,
};
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
