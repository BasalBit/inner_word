//! The GPUI rich text editor for The Writer.
//!
//! [`Editor`] is a GPUI entity that shows and edits a
//! [`writer_core::Document`] through a [`writer_core::EditorState`]. This
//! crate owns layout, painting, text input, the editing key bindings and
//! scrolling. Everything else (format menu, insert menu, link popover, find
//! and replace, Vim mode, the comments panel, the app shell) lives in other
//! crates and plugs in through the seams below.
//!
//! # Setup
//!
//! ```ignore
//! // once at startup, after writer_ui::theme::init and writer_ui::init
//! writer_editor::init(cx);
//!
//! // per document
//! let editor = cx.new(|cx| Editor::new(document, EditorConfig::full(), window, cx));
//! // in a parent's render
//! div().size_full().child(editor.clone())
//! ```
//!
//! [`init`] binds the editor's keys in the `"Editor"` key context
//! ([`actions::KEY_CONTEXT`]). [`commands`] lists the user-facing commands
//! for the command palette and the shortcuts overlay.
//!
//! # The seams
//!
//! | Need | API |
//! |---|---|
//! | Read the document and selection | [`Editor::state`], [`Editor::document`], [`Editor::selection`] |
//! | Change anything | [`Editor::edit`], [`Editor::edit_with`] (the only mutation path) |
//! | React to changes | `cx.subscribe(&editor, ..)` with [`EditorEvent`] |
//! | Configure | [`EditorConfig`], [`Editor::set_config`], [`Editor::update_config`], [`Editor::set_caret_style`], [`Editor::set_document`], [`Editor::toggle_page_view`], [`Editor::set_draw_as_active`] |
//! | See keys first | [`Editor::intercept_keys`] with an [`InterceptPriority`], returning a [`KeyDisposition`] |
//! | Refuse typed text | [`Editor::set_text_input_enabled`] |
//! | Join key dispatch with own bindings | [`Editor::set_key_context_value`], [`Editor::add_key_context_flag`], [`Editor::remove_key_context_entry`] |
//! | Where is it on screen | [`Editor::caret_bounds`], [`Editor::position_bounds`], [`Editor::range_bounds`], [`Editor::selection_bounds`], [`Editor::position_at_point`], [`Editor::viewport_bounds`], [`Editor::visible_range`] |
//! | Vertical movement over wrapped rows | [`Editor::visual_row`], [`Editor::move_by_visual_rows`], [`Editor::column_x`], [`Editor::rows_per_page`], [`Editor::visible_rows`], [`Editor::first_visible_row`], [`Editor::last_visible_row`] |
//! | Scroll | [`Editor::scroll_to_caret`], [`Editor::scroll_to_position`], [`Editor::scroll_by_rows`], [`Editor::scroll_by_pages`], [`Editor::scroll_by_pixels`] with [`Autoscroll`] |
//! | Float a view at the caret | [`Editor::show_popup`], [`Editor::hide_popup`] with [`PopupAnchor`], [`PopupPlacement`] |
//! | Mark ranges (find, Vim search) | [`Editor::set_highlights`], [`Editor::clear_highlights`] with [`HighlightStyle`] |
//! | Comments | [`EditorConfig::comments_visible`], [`Editor::set_active_comment`], [`Editor::set_hidden_comments`] |
//! | Status bar numbers | [`Editor::stats`], [`Editor::stats_pending`], [`Editor::selection_stats`], [`Editor::page_count`], [`Editor::caret_page`], [`Editor::layout_pending`] |
//! | Remember the view | [`Editor::view_state`], [`Editor::restore_view_state`] with [`ViewState`] |
//! | Focus | `Focusable` (`editor.focus_handle(cx)`), [`Editor::focus`], [`Editor::is_focused`] |
//! | A small input | [`EditorConfig::compact`], [`Editor::plain_text`], [`Editor::set_plain_text`], [`Editor::clear`], [`EditorEvent::Submitted`] |
//!
//! ## Mutation
//!
//! Outside code never holds `&mut EditorState`. It passes a closure to
//! [`Editor::edit`]; the editor runs it and then consumes writer-core's
//! change tracking, updates layout and statistics, emits events, scrolls
//! the caret into view and notifies. Positions are block index plus byte
//! offset and go stale with every edit: read [`Editor::selection`] again
//! afterwards.
//!
//! ## Key interception
//!
//! See [`Editor::intercept_keys`] and the `intercept` module source for the
//! mechanism (one `App::intercept_keystrokes` callback per editor, active
//! only while the editor's own focus handle is focused). Order for one key:
//! interceptors by descending [`InterceptPriority`], then key bindings,
//! then platform text input. A consumed key stops there.
//!
//! - A menu over the editor registers at [`InterceptPriority::OVERLAY`]
//!   while it is open, consumes `up` / `down` / `enter` / `tab` / `escape`
//!   and passes everything else, so typing keeps reaching the document.
//! - Vim registers at [`InterceptPriority::MODAL`]. In normal and visual
//!   mode it consumes every key that has no `cmd` modifier (those pass, so
//!   app and editor shortcuts keep working) and calls
//!   [`Editor::set_text_input_enabled`]`(false)` so nothing composed or
//!   dictated reaches the document. In insert mode it consumes only
//!   `escape` and control combinations and turns text input back on.
//!
//! ## Views
//!
//! Flow view is one centred column. Page view (the document's own
//! `page_view` setting, [`Editor::set_page_view`]) shows sheets at the size,
//! orientation and margins of the document's page setup: rows flow from
//! sheet to sheet, a block continues on the next sheet at a row boundary,
//! a page break starts a new sheet, headings stay with the text after
//! them. A window narrower than the page shows the page scaled down. All
//! geometry, the visual-row oracle and scrolling work the same in both
//! views.
//!
//! ## Geometry and visual rows
//!
//! Results are in window coordinates, `None` before the first layout, and
//! valid for the current document even right after an edit in the same
//! handler (queries lay out on demand). Vertical movement keeps a goal x
//! that is measured from the text column's left edge.
//!
//! # Module map (for whoever works on this crate)
//!
//! | Module | Contents |
//! |---|---|
//! | `editor` | the entity: state, config, mutation seam, interception and key context API, highlights and comments API, `Render` |
//! | `layout` | `DocumentLayout` (a slot per block with its measure, lazy placement, shaped layouts for what is on screen, background refinement, scroll anchor, word counts), `layout::block` (shaping, rows, hit testing), `layout::style` (fonts, colours, metrics), `layout::pages` (placement in flow view and on sheets) |
//! | `element` | `EditorElement`: measure, prepaint (frame plan), paint, listeners |
//! | `paint` | painting one visual row: glyphs, backgrounds, underlines |
//! | `decorations` | per block kind painting: markers, quote bar, code panel, rules |
//! | `input` | platform input handler (typing, IME) |
//! | `actions`, `handlers` | actions and key bindings; what each action does |
//! | `commands` | command descriptors for the palette |
//! | `movement` | the visual-row oracle |
//! | `mouse` | click, drag, checkboxes, links, wheel, scroll bar drag |
//! | `scroll` | scroll position, requests, autoscroll, the overlay scroll bar |
//! | `geometry` | positions to window rectangles and back |
//! | `highlights` | highlight layers |
//! | `popup` | popup hosting and placement |
//! | `intercept` | key interception registry |
//! | `compact` | compact mode: plain text helpers, height measurement |
//! | `stats` | statistics and page numbers |
//! | `view_state` | selection and scroll snapshot |
//!
//! # Long documents
//!
//! Nothing is laid out up front. A frame shapes the blocks in the viewport;
//! all others count with a height estimated from their length until a
//! background task has measured them, a few milliseconds at a time. For a
//! consumer this means: positions on screen are exact, while the scroll
//! range, [`Editor::page_count`] and the window y of positions far from the
//! viewport can still settle for a moment after a long document opens
//! ([`Editor::layout_pending`]). Word counts work the same way
//! ([`Editor::stats`], [`EditorEvent::StatsChanged`]).

pub mod actions;
mod commands;
mod compact;
mod config;
mod decorations;
mod editor;
mod element;
mod events;
mod geometry;
mod handlers;
mod highlights;
mod input;
mod intercept;
mod layout;
mod mouse;
mod movement;
mod paint;
mod popup;
mod scroll;
mod stats;
mod view_state;

#[cfg(test)]
mod tests;

pub use commands::{
    CATEGORY_BLOCK, CATEGORY_EDIT, CATEGORY_FORMAT, CATEGORY_INSERT, CATEGORY_LAYOUT, commands,
};
pub use config::{CaretStyle, CompactOptions, EditorConfig, EditorMode};
pub use editor::Editor;
#[doc(hidden)]
pub use editor::FramePerf;
pub use events::EditorEvent;
pub use highlights::HighlightStyle;
pub use intercept::{InterceptPriority, KeyDisposition};
pub use movement::{RowMove, VisualRow};
pub use popup::{PopupAnchor, PopupPlacement};
pub use scroll::Autoscroll;
pub use view_state::ViewState;

/// Installs the editor's key bindings (in the `"Editor"` key context). Call
/// once at startup, after the app's own context-free bindings and after
/// `writer_ui::init`.
pub fn init(cx: &mut gpui::App) {
    cx.bind_keys(actions::key_bindings());
}
