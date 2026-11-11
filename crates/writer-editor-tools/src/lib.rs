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
//! | Find and replace | `cmd-f`, `cmd-alt-f` | `enter` / `shift-enter` and `cmd-g` / `cmd-shift-g` step through the matches, `cmd-alt-c` case, `cmd-alt-w` whole word, `tab` between the fields, `enter` in the replace field replaces the current match, `cmd-enter` replaces all, `escape` closes and leaves the current match selected. |
//!
//! Opening a tool closes the one that is open (typing `/` leaves the find
//! bar alone, because it is part of typing). `escape` returns focus to the
//! editor.
//!
//! # Module map
//!
//! | Module | Contents |
//! |---|---|
//! | `tools` | [`EditorTools`]: state, action handlers, editor events, popup plumbing, render |
//! | `menu` | the popup list shared by the format menu, the insert menu and the font picker |
//! | `format_menu`, `insert_menu`, `link`, `font_picker`, `find` | one tool each, as `impl EditorTools` blocks |
//! | [`actions`] | actions and key bindings |
//! | `util` | today's date, URL clean-up |

pub mod actions;
mod commands;
mod find;
mod font_picker;
mod format_menu;
mod insert_menu;
mod link;
mod menu;
mod tools;
mod util;

pub use commands::{CATEGORY_FORMAT, CATEGORY_INSERT, CATEGORY_SEARCH, commands};
pub use find::{FIND_CURRENT_LAYER, FIND_LAYER};
pub use font_picker::{
    FontLibrary, RECENT_FONT_LIMIT, font_families, recent_fonts, set_font_families,
    set_recent_fonts,
};
pub use format_menu::TEXT_COLORS;
pub use menu::Mark;
pub use tools::{EditorTools, FindBarPlacement, MenuRow, ToolKind, ToolsEvent};
pub use util::{format_date, normalize_url, today_string};

/// Installs the key bindings of the tools. Call once at startup, after the
/// app's own context-free bindings, `writer_ui::init` and
/// `writer_editor::init`.
pub fn init(cx: &mut gpui::App) {
    cx.bind_keys(actions::key_bindings());
}
