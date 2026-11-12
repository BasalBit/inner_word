//! Events emitted by the [`crate::Editor`] entity.

use gpui::{Pixels, Point};
use writer_core::{Change, CommentId, Position, TextRange};

/// What a context menu over the text needs to know: sent with
/// [`EditorEvent::ContextMenuRequested`] when the user right-clicks in the
/// text (the secondary button, or `ctrl`-click, which macOS treats the
/// same) or presses `ctrl-enter` ([`crate::actions::OpenContextMenu`]).
/// The editor shows no menu itself.
///
/// By the time the event is emitted the editor has the keyboard and the
/// selection is what the menu's commands should act on:
///
/// - a right-click inside a non-empty selection keeps it;
/// - a right-click on a word anywhere else selects that word, as macOS
///   text views do;
/// - a right-click on anything else (a space, punctuation, a margin, an
///   empty block, a divider or page break, a to-do checkbox, the space
///   below the text) only puts the caret there;
/// - `ctrl-enter` leaves the selection alone (and scrolls the caret into
///   view first).
///
/// A `SelectionChanged` for the changed selection comes before this
/// event. Read [`crate::Editor::selection`] for the selection itself and
/// the document for the block ([`ContextMenuRequest::block`] is an index
/// into [`crate::Editor::document`]); `document.link_at(..)` gives the
/// text range of a link.
///
/// ```ignore
/// cx.subscribe_in(&editor, window, |this, editor, event: &EditorEvent, window, cx| {
///     if let EditorEvent::ContextMenuRequested(request) = event {
///         this.show_text_menu(request, window, cx); // anchored at request.position
///     }
/// })
/// ```
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ContextMenuRequest {
    /// Where to open the menu, in window coordinates: the pointer for a
    /// right-click; for the keyboard the bottom left of the caret (inside
    /// the editor's viewport).
    pub position: Point<Pixels>,
    /// The place in the document the request is about: for a right-click
    /// the caret position nearest to the pointer (what a plain click
    /// there would give), for the keyboard the caret.
    pub document_position: Position,
    /// Index of the block of [`ContextMenuRequest::document_position`].
    pub block: usize,
    /// The selection is not empty (after the right-click adjusted it).
    pub has_selection: bool,
    /// The URL of the link under the pointer; for the keyboard, of the
    /// link the caret is in or next to.
    pub link: Option<String>,
    /// The whole stretch of text that carries that link.
    pub link_range: Option<TextRange>,
    /// The comment threads anchored to the character under the pointer
    /// (for the keyboard: to the character before or after the caret),
    /// ordered by id. Threads hidden with
    /// [`crate::Editor::set_hidden_comments`] are left out; whether tints
    /// are drawn at all ([`crate::EditorConfig::comments_visible`]) makes
    /// no difference.
    pub comments: Vec<CommentId>,
    /// The pointer is on the checkbox of a to-do item (the item is
    /// [`ContextMenuRequest::block`]). The click did not toggle it. Always
    /// `false` for the keyboard.
    pub on_checkbox: bool,
    /// The request came from the keyboard (`ctrl-enter`), not from the
    /// pointer.
    pub from_keyboard: bool,
    /// The misspelled word the request is about: the spelling mark (see
    /// [`crate::Editor::set_spelling_marks`]) that
    /// [`ContextMenuRequest::document_position`] is in or at an end of, so
    /// for a right-click the marked word under the pointer and for the
    /// keyboard the marked word the caret is in, right before or right
    /// after. `None` when there is none there, when the pointer is on a
    /// to-do checkbox, and while spelling marks are not drawn
    /// ([`crate::Editor::set_spelling_marks_visible`]). A right-click on
    /// the word has selected it, as it selects any word; for the keyboard
    /// the selection is whatever it was.
    pub spelling_mark: Option<TextRange>,
}

/// What an [`crate::Editor`] tells its subscribers
/// (`cx.subscribe(&editor, |this, editor, event: &EditorEvent, cx| ..)`).
///
/// Events are emitted once per mutation, after the mutation is complete (see
/// [`crate::Editor::edit`]), in this order: `Edited`, `SettingsChanged`,
/// `SelectionChanged`; a `TextInput` comes before the `Edited` it belongs
/// to. The enum is `#[non_exhaustive]`: match with a wildcard arm.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum EditorEvent {
    /// Blocks or comment threads of the document changed (typing, a
    /// command, undo, an outside [`crate::Editor::edit`]). This is the
    /// autosave trigger. `change.blocks` is the changed block range,
    /// `change.comments` says whether threads or anchored text changed (the
    /// comments panel refreshes on it). Not emitted for pure selection or
    /// settings changes, nor by [`crate::Editor::set_document`].
    Edited {
        /// What changed, merged over the whole mutation.
        change: Change,
    },
    /// The selection differs from the one before the mutation.
    SelectionChanged,
    /// The keyboard came to the editor from outside (in an active
    /// window): to its text, or straight into its popup. Not emitted when
    /// focus returns from the popup to the text.
    Focused,
    /// The keyboard left the editor: neither its text nor anything in its
    /// popup (see [`crate::Editor::show_popup`]) is focused any more, or
    /// the window was deactivated. Not emitted when focus only moves from
    /// the text into the popup; [`crate::Editor::popup_focused`] and
    /// [`crate::Editor::is_focused`] tell the two places apart.
    Blurred,
    /// The document settings changed (page setup, typography, page view).
    /// Settings are saved with the document, so autosave listens to this
    /// too.
    SettingsChanged,
    /// [`crate::Editor::set_document`] replaced the whole document. No
    /// `Edited` is emitted for it. Consumers drop everything they derived
    /// from the old document (find matches, thread lists, Vim state).
    DocumentReplaced,
    /// Compact mode with submit-on-enter: the user pressed `enter`. The
    /// text is still in the editor; read it with
    /// [`crate::Editor::plain_text`] and empty it with
    /// [`crate::Editor::clear`].
    Submitted,
    /// [`crate::Editor::stats`] has a new result that was counted in the
    /// background (after a long document was opened or a change rewrote a
    /// very large part of it). Ordinary edits update the counts before
    /// `Edited` is emitted and do not emit this.
    StatsChanged,
    /// Text reached the document through the platform input handler: a
    /// typed character, the committed text of an IME composition or a dead
    /// key, the emoji palette, a system text replacement. Emitted once per
    /// insertion that changed the document, **before** the `Edited` (and
    /// `SelectionChanged`) of the same insertion, which always follow it:
    /// an `Edited` that was not announced by a `TextInput` did not come
    /// from text input. By the time a subscriber sees it the text is in
    /// the document and the caret is after it.
    ///
    /// Not emitted for updates of marked text during a composition (only
    /// for its commit), for input that was refused (read-only,
    /// [`crate::Editor::set_text_input_enabled`] off) or changed nothing,
    /// nor for text that arrives any other way (paste, `enter`, `tab`,
    /// [`crate::Editor::edit`]).
    TextInput {
        /// The text as the platform delivered it: before input rules and
        /// smart punctuation (the document may hold `“` where this says
        /// `"`), and before line-ending normalisation.
        text: String,
        /// `None` for plain typing (the text replaced the selection, if
        /// there was one) and for the commit of a composition (the text
        /// replaced the composition's own marked text, which was never
        /// reported). `Some(range)` when the platform named another range
        /// to replace: the accent menu replacing the letter that was just
        /// typed, a system text replacement. The range is in positions of
        /// the document before the insertion; the text went in at
        /// `range.start`.
        replaced: Option<TextRange>,
    },
    /// The background layout has measured every block: from now on (until
    /// the next change that leaves blocks to measure)
    /// [`crate::Editor::page_count`], [`crate::Editor::caret_page`] for
    /// far positions and the scroll range are exact, and
    /// [`crate::Editor::layout_pending`] is false. Until then they rest on
    /// estimated heights for the blocks that were never on screen, so a
    /// status bar that shows "page 3 of 1,290" for a book that was just
    /// opened or re-flowed is showing an estimate (say so: "of about
    /// 1,290") and should read the numbers again on this event.
    LayoutSettled,
    /// The user asked for the context menu of the text: a right-click
    /// (secondary button or `ctrl`-click) or `ctrl-enter`. The editor has
    /// already adjusted the selection as a macOS text view does; whoever
    /// shows menus opens one at [`ContextMenuRequest::position`]. See
    /// [`ContextMenuRequest`] for everything the event carries.
    ContextMenuRequested(ContextMenuRequest),
}
