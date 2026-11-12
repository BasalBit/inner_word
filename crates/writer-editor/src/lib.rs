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
//! | Change anything while asking about wrapped rows | [`Editor::edit_with_rows`] with a [`RowOracle`] |
//! | React to changes | `cx.subscribe(&editor, ..)` with [`EditorEvent`] |
//! | Learn what was typed | [`EditorEvent::TextInput`] |
//! | Configure | [`EditorConfig`], [`Editor::set_config`], [`Editor::update_config`], [`Editor::set_caret_style`], [`Editor::set_document`], [`Editor::toggle_page_view`], [`Editor::set_draw_as_active`] |
//! | Draw the caret away from the selection head | [`Editor::set_caret_override`], [`Editor::caret_override`], [`Editor::caret_position`] |
//! | See keys first | [`Editor::intercept_keys`] with an [`InterceptPriority`], returning a [`KeyDisposition`] |
//! | Refuse typed text | [`Editor::set_text_input_enabled`] |
//! | Join key dispatch with own bindings | [`Editor::set_key_context_value`], [`Editor::add_key_context_flag`], [`Editor::remove_key_context_entry`] |
//! | Where is it on screen | [`Editor::caret_bounds`], [`Editor::position_bounds`], [`Editor::range_bounds`], [`Editor::selection_bounds`], [`Editor::position_at_point`], [`Editor::viewport_bounds`], [`Editor::visible_range`] |
//! | Vertical movement over wrapped rows | [`Editor::visual_row`], [`Editor::move_by_visual_rows`], [`Editor::column_x`], [`Editor::rows_per_page`], [`Editor::visible_rows`], [`Editor::first_visible_row`], [`Editor::last_visible_row`] |
//! | Scroll | [`Editor::scroll_to_caret`], [`Editor::scroll_to_position`], [`Editor::scroll_by_rows`], [`Editor::scroll_by_pages`], [`Editor::scroll_by_pixels`] with [`Autoscroll`]; sideways [`Editor::scroll_sideways_by_pixels`] |
//! | Zoom the view | [`Editor::set_zoom`], [`Editor::set_zoom_at`], [`Editor::zoom_in`], [`Editor::zoom_out`], [`Editor::zoom_by`] with a [`Zoom`]; [`Editor::zoom`], [`Editor::zoom_percent`], [`Editor::set_view_zooms`], [`EditorEvent::ZoomChanged`] |
//! | Float a view at the caret | [`Editor::show_popup`], [`Editor::hide_popup`] with [`PopupAnchor`], [`PopupPlacement`] |
//! | Show a context menu for the text | [`EditorEvent::ContextMenuRequested`] with a [`ContextMenuRequest`] (right-click, `ctrl`-click, `ctrl-enter`) |
//! | Mark ranges (find, Vim search) | [`Editor::set_highlights`], [`Editor::clear_highlights`] with [`HighlightStyle`] |
//! | Comments | [`EditorConfig::comments_visible`], [`Editor::set_active_comment`], [`Editor::set_hidden_comments`] |
//! | Mark misspelled words (a spell checker) | [`Editor::set_spelling_marks`] per block and block revision with a [`SpellingMarksOutcome`], [`Editor::spelling_checked`], [`Editor::first_unchecked_spelling_block`], [`Editor::spelling_marks`], [`Editor::spelling_mark_at`], [`Editor::next_spelling_mark`], [`Editor::previous_spelling_mark`], [`Editor::spelling_mark_count`], [`Editor::invalidate_spelling_marks`], [`Editor::clear_spelling_marks`], [`Editor::set_spelling_marks_visible`], [`ContextMenuRequest::spelling_mark`] |
//! | Status bar numbers | [`Editor::stats`], [`Editor::stats_pending`], [`Editor::selection_stats`], [`Editor::page_count`], [`Editor::caret_page`], [`Editor::layout_pending`], [`EditorEvent::StatsChanged`], [`EditorEvent::LayoutSettled`] |
//! | Remember the view | [`Editor::view_state`], [`Editor::restore_view_state`] with [`ViewState`] |
//! | Focus | `Focusable` (`editor.focus_handle(cx)`), [`Editor::focus`], [`Editor::is_focused`] |
//! | A small input | [`EditorConfig::compact`], [`Editor::plain_text`], [`Editor::set_plain_text`], [`Editor::clear`], [`EditorEvent::Submitted`] |
//! | A view of rich text in a panel (an answer of the assistant) | [`CompactOptions::formatted`] with [`EditorConfig::read_only`]: sized like a small input, set with the typography of its document, headings at [`FORMATTED_HEADING_SCALE`] |
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
//! them. All geometry, the visual-row oracle and scrolling work the same in
//! both views.
//!
//! ## Zoom
//!
//! Each view has a zoom ([`Zoom`]: a percentage, or one that follows the
//! window). It changes how large the view is drawn and nothing of the
//! document: the layout is computed in the document's points and never
//! sees the zoom, so the same words stand on the same lines and pages at
//! every zoom of page view (the module `zoom` says how). A sheet that is
//! wider than its room scrolls sideways; flow view reflows instead.
//! Everything in the table above that is "in window coordinates" or "in
//! pixels" is at the zoom the view has, so whatever is anchored to the
//! text through it follows.
//!
//! ## Geometry and visual rows
//!
//! Results are in window coordinates, `None` before the first layout, and
//! valid for the current document even right after an edit in the same
//! handler (queries lay out on demand). Vertical movement keeps a goal x
//! that is measured from the text column's left edge.
//!
//! ## The caret at a soft wrap
//!
//! Where a paragraph wraps, one position is both the end of a row and the
//! start of the next. Positions (and everything that takes or returns one:
//! [`Editor::position_bounds`], [`Editor::visual_row`],
//! [`Editor::position_at_point`]) put it on the row that starts there. The
//! editor's own bar caret also remembers how it got there: after line end
//! (`cmd-right`, `end`), a click past the end of a wrapped row, or `up` /
//! `down` into a shorter row, it is drawn at the end of the row it is on,
//! and [`Editor::caret_bounds`], popups at the caret, autoscroll and the
//! next row-wise key follow that. Typing still inserts at the position.
//! This affinity is view state: it is not in the selection, it is dropped
//! when the head moves or the text changes, and a block or underline caret
//! or a caret override ([`Editor::set_caret_override`]) has none (line end
//! then stops on the row's last character, as the row oracle does).
//!
//! # Module map (for whoever works on this crate)
//!
//! | Module | Contents |
//! |---|---|
//! | `editor` | the entity: state, config, mutation seam, interception and key context API, highlights and comments API, `Render` |
//! | `layout` | `DocumentLayout` (a slot per block with its measure, lazy placement, shaped layouts for what is on screen, background refinement, scroll anchor, word counts), `layout::block` (shaping, rows, hit testing), `layout::wrap` (where rows break, the size of inline code), `layout::style` (fonts, colours, metrics), `layout::pages` (placement in flow view and on sheets) |
//! | `element` | `EditorElement`: measure, prepaint (frame plan), paint, listeners |
//! | `paint` | painting one visual row: glyphs, backgrounds, underlines |
//! | `decorations` | per block kind painting: markers, quote bar, code panel, rules |
//! | `input` | platform input handler (typing, IME) |
//! | `actions`, `handlers` | actions and key bindings; what each action does |
//! | `commands` | command descriptors for the palette |
//! | `movement` | the visual-row oracle |
//! | `mouse` | click, drag, checkboxes, links, the context menu request, wheel, scroll bar drag |
//! | `link_tip` | the tooltip of a link under the pointer |
//! | `scroll` | scroll position, requests, autoscroll, the overlay scroll bar |
//! | `geometry` | positions to window rectangles and back |
//! | `zoom` | the zoom of the two views: what it is, setting it, keeping the place |
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
//! ([`Editor::layout_pending`], then [`EditorEvent::LayoutSettled`]). Word
//! counts work the same way ([`Editor::stats`],
//! [`EditorEvent::StatsChanged`]).
//!
//! # The mouse
//!
//! Everything in the text can be done with the pointer alone: click,
//! shift-click, drag (which scrolls at the edges), double and triple
//! click, the to-do checkboxes, `cmd`-click on a link (resting on a link
//! shows its address and says so), the wheel and the scroll bar. A
//! right-click or `ctrl`-click, and `ctrl-enter` from the keyboard, ask
//! for the context menu: the editor adjusts the selection as a macOS text
//! view does and emits [`EditorEvent::ContextMenuRequested`] with a
//! [`ContextMenuRequest`]; whoever shows menus (writer-editor-tools)
//! opens one. The editor draws no menu itself.

pub mod actions;
mod adopt;
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
mod link_tip;
mod mouse;
mod movement;
mod paint;
mod popup;
mod scroll;
pub mod spelling;
mod stats;
mod view_state;
mod zoom;

#[cfg(test)]
mod tests;

pub use commands::{
    CATEGORY_BLOCK, CATEGORY_EDIT, CATEGORY_FORMAT, CATEGORY_INSERT, CATEGORY_LAYOUT, commands,
};
pub use config::{CaretStyle, CompactOptions, EditorConfig, EditorMode, FORMATTED_HEADING_SCALE};
pub use editor::Editor;
#[doc(hidden)]
pub use editor::FramePerf;
pub use events::{ContextMenuRequest, EditorEvent};
pub use highlights::HighlightStyle;
pub use intercept::{InterceptPriority, KeyDisposition};
pub use movement::{RowMove, RowOracle, VisualRow};
pub use popup::{PopupAnchor, PopupPlacement};
pub use scroll::Autoscroll;
pub use spelling::SpellingMarksOutcome;
pub use view_state::ViewState;
pub use zoom::{
    PAGE_VIEW_SCALE, ZOOM_FIT_MIN, ZOOM_MAX, ZOOM_MIN, ZOOM_STEPS, Zoom, step_above, step_below,
};

/// Installs the editor's key bindings (in the `"Editor"` key context). Call
/// once at startup, after the app's own context-free bindings and after
/// `writer_ui::init`.
pub fn init(cx: &mut gpui::App) {
    cx.bind_keys(actions::key_bindings());
}
