//! Events emitted by the [`crate::Editor`] entity.

use writer_core::{Change, TextRange};

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
}
